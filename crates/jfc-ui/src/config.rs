//! User-facing TOML config at `~/.config/jfc/config.toml`.
//!
//! Schema mirrors oh-my-opencode's `AgentOverrideConfigSchema` but trimmed to
//! the fields jfc currently understands. Everything is optional so a one-line
//! file like
//!
//! ```toml
//! [default]
//! model = "anthropic/claude-opus-4-7"
//! ```
//!
//! is valid. Unknown keys are tolerated (`#[serde(default)]` + struct `Default`)
//! so future versions of oh-my-opencode adding fields don't make our parser
//! reject existing user configs.
//!
//! Provider lookup is left to whoever consumes the resolved string — both bare
//! ids (`"claude-opus-4-7"`) and prefixed ids (`"anthropic/claude-opus-4-7"`)
//! pass through verbatim. This module is purely about *which* string to feed
//! to the existing `Provider::stream` pipeline; wiring that string into the
//! provider call site is intentionally out of scope here.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level config. `default` applies to the primary chat agent and any agent
/// not listed under `[agents.*]`; per-agent entries override individual fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub default: AgentConfig,
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
}

/// Per-agent overrides. Every field is optional; missing fields cascade up to
/// `[default]` (or to compiled-in fallbacks in the eventual consumer).
///
/// Field naming uses snake_case to match the TOML example in the schema doc;
/// the upstream Zod schema mixes camelCase (`maxTokens`, `disallowedTools`)
/// and snake_case (`fallback_models`, `prompt_append`), but TOML idiomatically
/// prefers snake_case so we normalize.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub fallback_models: Vec<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub thinking_budget: Option<u32>,
}

/// Canonical path to the config file. Always returned (even if the file
/// doesn't exist) so `/config path` can show the user where to create it.
///
/// Uses `dirs::config_dir()` (XDG `~/.config` on Linux, `~/Library/Application
/// Support` on macOS, `%APPDATA%` on Windows). When `dirs` returns `None`
/// (extremely rare — e.g. no `$HOME`), we fall back to a relative `./jfc/...`
/// path so callers always have *something* to display.
pub fn config_path() -> PathBuf {
    let path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jfc")
        .join("config.toml");
    tracing::trace!(target: "jfc::config", path = %path.display(), "resolved config path");
    path
}

/// Load `~/.config/jfc/config.toml`, or return `Config::default()` on any
/// failure (missing file, permission error, parse error). On parse errors we
/// log a warning via `tracing::warn!` so the user sees their typo'd file is
/// being silently ignored instead of dying mid-startup with a panic.
///
/// **Not unit-tested:** this function reads the live filesystem. Pure parsing
/// is exercised through `Config`'s `Deserialize` impl in the test module
/// below — feed it a string with `toml::from_str::<Config>` and verify the
/// fields land. Filesystem-level integration is left to manual testing /
/// the eventual startup integration ticket.
pub fn load() -> Config {
    let path = config_path();
    tracing::info!(target: "jfc::config", path = %path.display(), "loading config");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(target: "jfc::config", "config file not found — using defaults");
            return Config::default();
        }
        Err(e) => {
            tracing::warn!(
                target: "jfc::config",
                path = %path.display(),
                error = %e,
                "failed to read config file — using defaults"
            );
            return Config::default();
        }
    };
    match toml::from_str::<Config>(&raw) {
        Ok(cfg) => {
            tracing::debug!(
                target: "jfc::config",
                default_model = ?cfg.default.model,
                agent_count = cfg.agents.len(),
                "config loaded successfully"
            );
            cfg
        }
        Err(e) => {
            tracing::warn!(
                target: "jfc::config",
                path = %path.display(),
                error = %e,
                "failed to parse config — using defaults"
            );
            Config::default()
        }
    }
}

/// Resolve which model id should be used for a given agent, with a four-step
/// cascade:
///
/// 1. `[agents.<name>].model` — explicit per-agent pin
/// 2. `[agents.<name>].fallback_models[0]` — first fallback when `model` unset
/// 3. `[default].model` — global default
/// 4. `None` — nothing configured, caller decides (usually `ANTHROPIC_MODEL`
///    env var or hardcoded constant)
///
/// `agent_name = None` skips steps 1 & 2 and goes straight to default. This is
/// the path the primary chat agent takes since it has no per-agent override
/// section.
pub fn resolve_model(cfg: &Config, agent_name: Option<&str>) -> Option<String> {
    let result = if let Some(name) = agent_name {
        if let Some(agent) = cfg.agents.get(name) {
            if let Some(m) = agent.model.as_ref().filter(|s| !s.is_empty()) {
                Some(m.clone())
            } else if let Some(m) = agent.fallback_models.first() {
                Some(m.clone())
            } else {
                cfg.default.model.clone().filter(|s| !s.is_empty())
            }
        } else {
            cfg.default.model.clone().filter(|s| !s.is_empty())
        }
    } else {
        cfg.default.model.clone().filter(|s| !s.is_empty())
    };
    tracing::debug!(
        target: "jfc::config",
        agent_name = ?agent_name,
        resolved_model = ?result,
        "resolve_model"
    );
    result
}

/// Tools the named agent should NOT have access to. Returns `&[]` when the
/// agent isn't in the config at all (so callers can union this with
/// compiled-in defaults without an extra `None` branch).
pub fn agent_disallowed<'a>(cfg: &'a Config, agent_name: &str) -> &'a [String] {
    cfg.agents
        .get(agent_name)
        .map(|a| a.disallowed_tools.as_slice())
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Config {
        toml::from_str::<Config>(src).expect("expected valid toml")
    }

    #[test]
    fn parse_minimal_config_normal() {
        let cfg = parse(r#"
[default]
model = "x"
"#);
        assert_eq!(cfg.default.model.as_deref(), Some("x"));
        assert!(cfg.agents.is_empty());
        assert!(cfg.default.fallback_models.is_empty());
        assert_eq!(cfg.default.temperature, None);
    }

    #[test]
    fn parse_full_config_normal() {
        let cfg = parse(r#"
[default]
model = "anthropic/claude-opus-4-7"
fallback_models = ["openai/gpt-5"]
temperature = 0.7

[agents.code-reviewer]
model = "anthropic/claude-sonnet-4-6"
fallback_models = ["anthropic/claude-haiku-4-5"]
temperature = 0.1
disallowed_tools = ["Bash", "Write"]
description = "Reviews diffs for security/perf"

[agents.formatter]
model = "anthropic/claude-haiku-4-5"
mode = "subagent"
"#);
        // [default]
        assert_eq!(
            cfg.default.model.as_deref(),
            Some("anthropic/claude-opus-4-7")
        );
        assert_eq!(cfg.default.fallback_models, vec!["openai/gpt-5"]);
        assert_eq!(cfg.default.temperature, Some(0.7));

        // [agents.code-reviewer]
        let reviewer = cfg
            .agents
            .get("code-reviewer")
            .expect("code-reviewer agent");
        assert_eq!(
            reviewer.model.as_deref(),
            Some("anthropic/claude-sonnet-4-6")
        );
        assert_eq!(
            reviewer.fallback_models,
            vec!["anthropic/claude-haiku-4-5"]
        );
        assert_eq!(reviewer.temperature, Some(0.1));
        assert_eq!(reviewer.disallowed_tools, vec!["Bash", "Write"]);
        assert_eq!(
            reviewer.description.as_deref(),
            Some("Reviews diffs for security/perf")
        );

        // [agents.formatter]
        let fmt = cfg.agents.get("formatter").expect("formatter agent");
        assert_eq!(fmt.model.as_deref(), Some("anthropic/claude-haiku-4-5"));
        assert_eq!(fmt.mode.as_deref(), Some("subagent"));
    }

    #[test]
    fn resolve_model_uses_agent_override_normal() {
        let cfg = parse(r#"
[default]
model = "B"

[agents.code-reviewer]
model = "A"
"#);
        assert_eq!(
            resolve_model(&cfg, Some("code-reviewer")),
            Some("A".to_owned())
        );
    }

    #[test]
    fn resolve_model_falls_through_to_default_normal() {
        let cfg = parse(r#"
[default]
model = "B"

[agents.code-reviewer]
temperature = 0.1
"#);
        assert_eq!(
            resolve_model(&cfg, Some("code-reviewer")),
            Some("B".to_owned())
        );
    }

    #[test]
    fn resolve_model_unknown_agent_uses_default_normal() {
        let cfg = parse(r#"
[default]
model = "B"
"#);
        assert_eq!(
            resolve_model(&cfg, Some("does-not-exist")),
            Some("B".to_owned())
        );
    }

    #[test]
    fn resolve_model_uses_fallback_when_no_model_robust() {
        // Agent has no `model` field but a non-empty `fallback_models` list →
        // step 2 of the cascade returns the first fallback, NOT the default.
        let cfg = parse(r#"
[default]
model = "default-model"

[agents.formatter]
fallback_models = ["fallback-A", "fallback-B"]
"#);
        assert_eq!(
            resolve_model(&cfg, Some("formatter")),
            Some("fallback-A".to_owned())
        );
    }

    #[test]
    fn resolve_model_returns_none_when_nothing_configured_robust() {
        let cfg = Config::default();
        assert_eq!(resolve_model(&cfg, None), None);
        assert_eq!(resolve_model(&cfg, Some("anything")), None);
    }

    #[test]
    fn agent_disallowed_returns_list_normal() {
        let cfg = parse(r#"
[agents.code-reviewer]
disallowed_tools = ["Bash", "Write"]
"#);
        assert_eq!(
            agent_disallowed(&cfg, "code-reviewer"),
            &["Bash".to_owned(), "Write".to_owned()]
        );
    }

    #[test]
    fn agent_disallowed_unknown_agent_returns_empty_robust() {
        let cfg = Config::default();
        assert!(agent_disallowed(&cfg, "ghost").is_empty());

        // Known agent, no disallowed_tools key → still empty slice.
        let cfg2 = parse(r#"
[agents.formatter]
model = "x"
"#);
        assert!(agent_disallowed(&cfg2, "formatter").is_empty());
    }

    #[test]
    fn parse_malformed_toml_returns_default_robust() {
        // toml::from_str on garbage MUST surface an Err — `load()` swallows
        // that into the default; here we just verify the parser doesn't panic
        // and that the deserializer reports a clean error rather than a panic
        // or accidental success.
        let bad = "this is = = not toml [ [ [";
        let result = toml::from_str::<Config>(bad);
        assert!(result.is_err(), "garbage toml must not parse");

        // And the corresponding swallow path: a Config built fresh from
        // Default has no agents and no default model.
        let cfg = Config::default();
        assert!(cfg.agents.is_empty());
        assert!(cfg.default.model.is_none());
    }
}
