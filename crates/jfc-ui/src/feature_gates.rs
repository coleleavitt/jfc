//! v132 feature-gate framework.
//!
//! v132 ships a constellation of per-feature flags identified by codename
//! (`harbor`, `harrier`, `kestrel`, `meadow`, `prism`, `siskin`, `thimble`,
//! `ribbon`, `finch`). Each flips a single behavior; together they form the
//! product surface. The codenames map onto the `tengu_slate_<name>` telemetry
//! events the v132 binary emits when a feature decision is logged.
//!
//! `slate.rs` already exists in jfc but is a model *router* — a different
//! concept that uses the same codenames as labels. This module is the actual
//! v132 feature-gate framework: a single source of truth for which behaviors
//! are enabled, defaultable from `~/.config/jfc/feature_gates.json` and
//! overridable per-session via `/feature <gate> on|off`.
//!
//! Adding a new gate:
//! 1. Add a variant to `FeatureGate` with a doc comment quoting the v132
//!    behavior it controls.
//! 2. Set its default in `default_for`.
//! 3. Surface it in `system_prompt_section` if the LLM needs to know.
//! 4. Read it from any execution site via `is_enabled(gate)`.

use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeatureGate {
    /// REPL-style experience: persistent context across turns, inline
    /// continuation prompts, no full redraws between turns. v132's primary
    /// interactive mode.
    Harbor,
    /// Investigate-first nudge: the model spends up to ~1 minute on
    /// read-only investigation (Read/Grep/Glob/git log) before asking the
    /// user a clarifying question, when the task scope is bounded.
    Harrier,
    /// Permission policy autopilot: when a tool's permission rule matches
    /// a non-interactive ALLOW or DENY, skip the prompt entirely and log
    /// the decision. Only triggers when rules are explicit.
    Kestrel,
    /// Fleet/swarm orchestration: surface multi-agent dispatch in the UI
    /// (lanes, status badges, per-agent token counts).
    Meadow,
    /// Progress UI: spinner with per-tool live status, ETA, token counts,
    /// and elapsed wall-clock. Disable for plain text.
    Prism,
    /// Memory survey: two-phase recall (cheap bulk listing + targeted
    /// synthesis) injected into the system prompt. Disable when the user
    /// runs `/memory off` or for stateless one-shots.
    Siskin,
    /// Auto-memory extraction: at end-of-turn, scan the assistant text for
    /// candidate memory facts ("user prefers X", "always do Y") and queue
    /// them for the next /memorize gesture.
    Thimble,
    /// Ribbon UI: header strip showing model, mode, cwd, branch, cost.
    /// Disable on terminals with no top row (split panes, tiny windows).
    Ribbon,
    /// Finch onboarding: first-run UI showing keybindings, slash command
    /// catalog, and a guided tour. Auto-disables after first successful
    /// turn.
    Finch,
    /// Batch tool approval: when multiple tools are queued for approval,
    /// show a single combined prompt with per-tool approve/deny + an
    /// approve-all option, instead of one prompt per tool.
    Tern,
    /// Mid-stream Bash output to model: stream the bash subprocess
    /// stdout into the LLM context as it arrives so the user can
    /// interrupt mid-command. Off by default (token-heavy).
    Marsh,
}

impl FeatureGate {
    pub const ALL: &'static [FeatureGate] = &[
        FeatureGate::Harbor,
        FeatureGate::Harrier,
        FeatureGate::Kestrel,
        FeatureGate::Meadow,
        FeatureGate::Prism,
        FeatureGate::Siskin,
        FeatureGate::Thimble,
        FeatureGate::Ribbon,
        FeatureGate::Finch,
        FeatureGate::Tern,
        FeatureGate::Marsh,
    ];

    pub fn codename(self) -> &'static str {
        match self {
            Self::Harbor => "harbor",
            Self::Harrier => "harrier",
            Self::Kestrel => "kestrel",
            Self::Meadow => "meadow",
            Self::Prism => "prism",
            Self::Siskin => "siskin",
            Self::Thimble => "thimble",
            Self::Ribbon => "ribbon",
            Self::Finch => "finch",
            Self::Tern => "tern",
            Self::Marsh => "marsh",
        }
    }

    pub fn from_codename(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|g| g.codename() == s)
    }

    /// Default state for a gate. Mirrors v132 ship defaults — most are on,
    /// `Finch` is conditional on first-run.
    pub fn default_for(self) -> bool {
        match self {
            Self::Harbor => true,
            Self::Harrier => true,
            Self::Kestrel => true,
            Self::Meadow => true,
            Self::Prism => true,
            Self::Siskin => true,
            Self::Thimble => true,
            Self::Ribbon => true,
            Self::Finch => false,
            Self::Tern => true,
            Self::Marsh => false,
        }
    }

    /// One-line human-readable description for the `/feature` UI.
    pub fn description(self) -> &'static str {
        match self {
            Self::Harbor => "REPL mode: persistent context across turns",
            Self::Harrier => "Investigate-first: explore before asking",
            Self::Kestrel => "Permission autopilot: skip prompts on explicit rules",
            Self::Meadow => "Fleet UI: surface multi-agent dispatch",
            Self::Prism => "Live progress UI with per-tool status",
            Self::Siskin => "Two-phase memory recall in system prompt",
            Self::Thimble => "Auto-extract memory candidates from assistant text",
            Self::Ribbon => "Header ribbon with model/mode/cwd/branch/cost",
            Self::Finch => "First-run onboarding tour",
            Self::Tern => "Batch tool approval — one prompt for all queued tools",
            Self::Marsh => "Mid-stream bash output fed to model context",
        }
    }
}

static OVERRIDES: RwLock<Option<HashMap<FeatureGate, bool>>> = RwLock::new(None);

/// Check whether a gate is enabled for this process. Returns the override
/// if `set` was called, else the default.
pub fn is_enabled(gate: FeatureGate) -> bool {
    OVERRIDES
        .read()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(&gate).copied()))
        .unwrap_or_else(|| gate.default_for())
}

/// Set a gate's value for this process (e.g. via `/feature harrier off`).
pub fn set(gate: FeatureGate, enabled: bool) {
    let Ok(mut guard) = OVERRIDES.write() else {
        return;
    };
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(gate, enabled);
    tracing::info!(
        target: "jfc::feature_gates",
        codename = gate.codename(),
        enabled,
        "feature gate set"
    );
}

/// Render a status block for the system prompt so the model knows which
/// behaviors are live this turn. Suppressed entirely if every gate is at
/// its default (the model already knows the defaults from training).
pub fn system_prompt_section() -> Option<String> {
    let overrides = OVERRIDES.read().ok()?;
    let map = overrides.as_ref()?;
    if map.is_empty() {
        return None;
    }
    let mut deviations = Vec::new();
    for (&gate, &enabled) in map {
        if enabled != gate.default_for() {
            deviations.push((gate, enabled));
        }
    }
    if deviations.is_empty() {
        return None;
    }
    deviations.sort_by_key(|(g, _)| g.codename());
    let mut out = String::from("\n\n## Feature gates (deviations from default)\n\n");
    for (gate, enabled) in deviations {
        out.push_str(&format!(
            "- `{}`: **{}** ({})\n",
            gate.codename(),
            if enabled { "ON" } else { "OFF" },
            gate.description()
        ));
    }
    Some(out)
}

// ─── Marsh shared buffer ────────────────────────────────────────────────────
//
// Process-global slot the streaming bash tool fills with chunks; the
// next outbound `stream_response` drains it and prepends the body as a
// `<system-reminder>` so the model sees what bash printed since the
// previous turn. This is the seam — full mid-stream injection (sending
// chunks to the API while the model is mid-stream) requires Anthropic
// to emit a corresponding tool_result content block delta, which their
// API supports but jfc's provider layer doesn't yet wire.

use std::sync::Mutex;

static MARSH_BUFFER: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Append a chunk produced by the streaming bash tool. Capped at 200
/// lines so a chatty command doesn't bloat the next prompt.
pub fn marsh_push(line: impl Into<String>) {
    let Ok(mut guard) = MARSH_BUFFER.lock() else {
        return;
    };
    guard.push(line.into());
    let len = guard.len();
    if len > 200 {
        guard.drain(0..(len - 200));
    }
}

/// Drain and return the buffered chunks. Caller wraps them in a
/// `<system-reminder>` for the next outbound prompt.
pub fn marsh_drain() -> Vec<String> {
    let Ok(mut guard) = MARSH_BUFFER.lock() else {
        return Vec::new();
    };
    std::mem::take(&mut *guard)
}

#[cfg(test)]
fn clear_for_test() {
    if let Ok(mut g) = OVERRIDES.write() {
        *g = Some(HashMap::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_stable_normal() {
        clear_for_test();
        assert!(is_enabled(FeatureGate::Harbor));
        assert!(is_enabled(FeatureGate::Siskin));
        assert!(!is_enabled(FeatureGate::Finch));
    }

    #[test]
    fn set_overrides_default_normal() {
        clear_for_test();
        set(FeatureGate::Harrier, false);
        assert!(!is_enabled(FeatureGate::Harrier));
        set(FeatureGate::Finch, true);
        assert!(is_enabled(FeatureGate::Finch));
    }

    #[test]
    fn from_codename_round_trip_normal() {
        for &gate in FeatureGate::ALL {
            assert_eq!(FeatureGate::from_codename(gate.codename()), Some(gate));
        }
    }

    #[test]
    fn from_codename_unknown_is_none_robust() {
        assert!(FeatureGate::from_codename("not-a-gate").is_none());
        assert!(FeatureGate::from_codename("").is_none());
    }

    #[test]
    fn system_prompt_section_empty_when_no_deviations_robust() {
        clear_for_test();
        assert!(system_prompt_section().is_none());
    }

    #[test]
    fn system_prompt_section_lists_deviations_normal() {
        clear_for_test();
        set(FeatureGate::Harrier, false);
        set(FeatureGate::Finch, true);
        let section = system_prompt_section().unwrap();
        assert!(section.contains("harrier"));
        assert!(section.contains("OFF"));
        assert!(section.contains("finch"));
        assert!(section.contains("ON"));
    }

    #[test]
    fn system_prompt_section_skips_default_overrides_robust() {
        clear_for_test();
        // Setting a gate to its default value should not surface as a deviation.
        set(FeatureGate::Harbor, true);
        assert!(system_prompt_section().is_none());
    }
}
