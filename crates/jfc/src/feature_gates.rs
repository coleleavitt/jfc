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
    /// v137 `/goal` command: set a condition and keep working until it's
    /// met. Session-scoped Stop hook blocks completion until the goal holds.
    /// Clearable via `/goal clear`.
    MapleTide,
    /// v137 simple system prompt: certain models receive a reduced system
    /// prompt instead of the full instruction set. Controlled server-side
    /// via a model list; this gate enables the local opt-in path.
    VelvetCascade,
    /// v137 autocompact gating: additional pre-compact checks for
    /// background sessions to avoid compacting during active tool runs.
    BasaltSpur,
    /// v137 dynamic notification banner: server can push a transient
    /// announcement (maintenance window, feature rollout) that surfaces
    /// as a high-priority toast.
    PorchBell,
    /// v152 streaming tool execution: begin executing auto-approved tools
    /// as soon as their content_block_stop arrives, without waiting for the
    /// model's full response to finish (message_stop). Safe because tool_use
    /// blocks are complete at content_block_stop — name, ID, and input JSON
    /// are all finalized.
    StreamingToolExec,
    /// v152 destructive command warning: show ⚠ DESTRUCTIVE label in the
    /// permission prompt when the bash command matches a known-dangerous
    /// pattern (rm -rf, git push --force, dd, sudo, etc.).
    DestructiveWarn,
    /// Show one-time notice that auto is the default permission mode.
    /// When enabled, on session start check if `~/.config/jfc/auto_nudge_seen`
    /// exists. If not, display a notice and create the marker file.
    AutoDefaultNudge,
    /// Claude Code 2.1.153 server-side advisor rollout gate
    /// (`tengu_sage_compass2`). Enables the Anthropic `advisor` server tool
    /// when the active model supports it.
    TenguSageCompass2,
    /// Claude Code 2.1.159 Pewter Owl header rollout
    /// (`pewter_owl_header`). Adds the `narration_summaries` beta header
    /// for interactive Anthropic-native requests.
    PewterOwlHeader,
    /// Claude Code 2.1.159 Pewter Owl tool rollout (`pewter_owl_tool`).
    /// Enables `SendUserMessage` outside strict brief mode with the lighter
    /// Pewter Owl prompt.
    PewterOwlTool,
    /// Claude Code 2.1.159 Pewter Owl brief rollout (`pewter_owl_brief`).
    /// Forces brief-mode visibility and `SendUserMessage` availability.
    PewterOwlBrief,
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
        FeatureGate::MapleTide,
        FeatureGate::VelvetCascade,
        FeatureGate::BasaltSpur,
        FeatureGate::PorchBell,
        FeatureGate::StreamingToolExec,
        FeatureGate::DestructiveWarn,
        FeatureGate::AutoDefaultNudge,
        FeatureGate::TenguSageCompass2,
        FeatureGate::PewterOwlHeader,
        FeatureGate::PewterOwlTool,
        FeatureGate::PewterOwlBrief,
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
            Self::MapleTide => "maple-tide",
            Self::VelvetCascade => "velvet-cascade",
            Self::BasaltSpur => "basalt-spur",
            Self::PorchBell => "porch-bell",
            Self::StreamingToolExec => "streaming-tool-exec",
            Self::DestructiveWarn => "destructive-warn",
            Self::AutoDefaultNudge => "auto-default-nudge",
            Self::TenguSageCompass2 => "tengu_sage_compass2",
            Self::PewterOwlHeader => "pewter_owl_header",
            Self::PewterOwlTool => "pewter_owl_tool",
            Self::PewterOwlBrief => "pewter_owl_brief",
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
            Self::MapleTide => false,
            Self::VelvetCascade => false,
            Self::BasaltSpur => false,
            Self::PorchBell => false,
            Self::StreamingToolExec => false,
            Self::DestructiveWarn => true,
            Self::AutoDefaultNudge => false,
            Self::TenguSageCompass2 => false,
            Self::PewterOwlHeader => false,
            Self::PewterOwlTool => false,
            Self::PewterOwlBrief => false,
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
            Self::MapleTide => "/goal: keep working until condition is met",
            Self::VelvetCascade => "Simple system prompt for select models",
            Self::BasaltSpur => "Extra pre-compact checks for background sessions",
            Self::PorchBell => "Dynamic server notification banner",
            Self::StreamingToolExec => "Begin tool execution before model finishes streaming",
            Self::DestructiveWarn => "Show ⚠ DESTRUCTIVE warning before dangerous commands",
            Self::AutoDefaultNudge => {
                "Show one-time notice that auto is the default permission mode"
            }
            Self::TenguSageCompass2 => "Enable the Anthropic server-side advisor tool",
            Self::PewterOwlHeader => "Enable Pewter Owl narration summary beta headers",
            Self::PewterOwlTool => "Enable Pewter Owl SendUserMessage tool prompt",
            Self::PewterOwlBrief => "Enable Pewter Owl brief-only display mode",
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

pub fn pewter_owl_header_enabled(model: &str, non_interactive: bool) -> bool {
    pewter_owl_gate_enabled(FeatureGate::PewterOwlHeader, model, non_interactive)
}

pub fn pewter_owl_tool_enabled(model: &str, non_interactive: bool) -> bool {
    pewter_owl_gate_enabled(FeatureGate::PewterOwlTool, model, non_interactive)
}

pub fn pewter_owl_brief_enabled(model: &str, non_interactive: bool) -> bool {
    pewter_owl_gate_enabled(FeatureGate::PewterOwlBrief, model, non_interactive)
}

fn pewter_owl_gate_enabled(gate: FeatureGate, model: &str, non_interactive: bool) -> bool {
    if env_falsey("CLAUDE_CODE_PEWTER_OWL") || env_falsey("JFC_PEWTER_OWL") {
        return false;
    }
    if env_truthy("CLAUDE_CODE_PEWTER_OWL") || env_truthy("JFC_PEWTER_OWL") {
        return true;
    }
    if non_interactive {
        return false;
    }
    if let Some(filter) = pewter_owl_model_filter()
        && !canonical_model(model).contains(&filter)
    {
        return false;
    }
    is_enabled(gate)
}

fn pewter_owl_model_filter() -> Option<String> {
    std::env::var("JFC_PEWTER_OWL_MODEL")
        .ok()
        .or_else(|| std::env::var("CLAUDE_CODE_PEWTER_OWL_MODEL").ok())
        .map(|v| canonical_model(&v))
        .filter(|v| !v.is_empty())
}

fn canonical_model(model: &str) -> String {
    model.trim().to_ascii_lowercase().replace('_', "-")
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .is_some_and(|v| matches_bool(&v, &["1", "true", "yes", "on"]))
}

fn env_falsey(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .is_some_and(|v| matches_bool(&v, &["0", "false", "no", "off"]))
}

fn matches_bool(value: &str, accepted: &[&str]) -> bool {
    let value = value.trim().to_ascii_lowercase();
    accepted.iter().any(|candidate| value == *candidate)
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

    #[serial_test::serial]
    #[test]
    fn defaults_are_stable_normal() {
        clear_for_test();
        assert!(is_enabled(FeatureGate::Harbor));
        assert!(is_enabled(FeatureGate::Siskin));
        assert!(!is_enabled(FeatureGate::Finch));
        assert!(!is_enabled(FeatureGate::PewterOwlHeader));
        assert!(!is_enabled(FeatureGate::PewterOwlTool));
        assert!(!is_enabled(FeatureGate::PewterOwlBrief));
    }

    #[serial_test::serial]
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

    #[serial_test::serial]
    #[test]
    fn system_prompt_section_empty_when_no_deviations_robust() {
        clear_for_test();
        assert!(system_prompt_section().is_none());
    }

    #[serial_test::serial]
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

    #[serial_test::serial]
    #[test]
    fn system_prompt_section_skips_default_overrides_robust() {
        clear_for_test();
        // Setting a gate to its default value should not surface as a deviation.
        set(FeatureGate::Harbor, true);
        assert!(system_prompt_section().is_none());
    }

    #[serial_test::serial]
    #[test]
    fn pewter_owl_env_force_enables_all_gates_normal() {
        clear_for_test();
        unsafe {
            std::env::set_var("CLAUDE_CODE_PEWTER_OWL", "1");
            std::env::remove_var("JFC_PEWTER_OWL");
            std::env::remove_var("JFC_PEWTER_OWL_MODEL");
            std::env::remove_var("CLAUDE_CODE_PEWTER_OWL_MODEL");
        }
        assert!(pewter_owl_header_enabled("claude-opus-4-7", true));
        assert!(pewter_owl_tool_enabled("claude-opus-4-7", true));
        assert!(pewter_owl_brief_enabled("claude-opus-4-7", true));
        unsafe { std::env::remove_var("CLAUDE_CODE_PEWTER_OWL") };
    }

    #[serial_test::serial]
    #[test]
    fn pewter_owl_env_force_disable_wins_robust() {
        clear_for_test();
        set(FeatureGate::PewterOwlHeader, true);
        unsafe {
            std::env::set_var("CLAUDE_CODE_PEWTER_OWL", "0");
            std::env::remove_var("JFC_PEWTER_OWL");
            std::env::remove_var("JFC_PEWTER_OWL_MODEL");
            std::env::remove_var("CLAUDE_CODE_PEWTER_OWL_MODEL");
        }
        assert!(!pewter_owl_header_enabled("claude-opus-4-7", false));
        unsafe { std::env::remove_var("CLAUDE_CODE_PEWTER_OWL") };
    }

    #[serial_test::serial]
    #[test]
    fn pewter_owl_gate_honors_model_filter_normal() {
        clear_for_test();
        set(FeatureGate::PewterOwlTool, true);
        unsafe {
            std::env::remove_var("CLAUDE_CODE_PEWTER_OWL");
            std::env::remove_var("JFC_PEWTER_OWL");
            std::env::set_var("JFC_PEWTER_OWL_MODEL", "opus-4-8");
        }
        assert!(pewter_owl_tool_enabled("claude-opus-4-8", false));
        assert!(!pewter_owl_tool_enabled("claude-sonnet-4-6", false));
        unsafe { std::env::remove_var("JFC_PEWTER_OWL_MODEL") };
    }
}
