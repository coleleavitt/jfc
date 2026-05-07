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
//! ## Model specifier format
//!
//! The `model` field accepts two forms (see `provider::ModelSpec`):
//!
//! - **Qualified**: `"provider/model-id"` — routes directly to the named provider.
//!   Examples: `"openwebui/bedrock-claude-4-6-opus"`, `"anthropic/claude-opus-4-7"`
//!
//! - **Bare**: `"model-id"` — provider resolved by heuristic (static catalogue
//!   match, then OpenWebUI catch-all for non-`claude-` prefixed ids).
//!   Examples: `"claude-opus-4-7"`, `"bedrock-claude-4-6-opus"`
//!
//! The qualified form eliminates the class of bugs where model and provider are
//! resolved independently and end up mismatched (e.g. a Bedrock model id
//! routed to the Anthropic API, yielding a 404).

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
    /// Domain-based model routing (e.g. "visual-engineering" → specific model).
    #[serde(default)]
    pub categories: HashMap<String, CategoryConfig>,
    /// Permission automation rules (auto-approve/deny tool calls by pattern).
    #[serde(default)]
    pub permission_automation: Option<PermissionAutomationConfig>,
    /// Background task concurrency limits.
    #[serde(default)]
    pub background_task: Option<BackgroundTaskConfig>,
    /// Argus auto-review configuration.
    #[serde(default)]
    pub argus_auto_review: Option<ArgusAutoReviewConfig>,
    /// MCP (Model Context Protocol) server definitions.
    #[serde(default)]
    pub mcp: HashMap<String, McpServerConfig>,
    /// Agents to disable (by name).
    #[serde(default)]
    pub disabled_agents: Vec<String>,
    /// Tools to disable globally (by name).
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    /// Experimental feature flags.
    #[serde(default)]
    pub experimental: Option<ExperimentalConfig>,
}

/// Category-based model routing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CategoryConfig {
    pub model: Option<String>,
    #[serde(default)]
    pub prompt_append: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

/// Permission automation rules in main config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PermissionAutomationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<PermissionRuleEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PermissionRuleEntry {
    pub action: String,
    pub tool: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Background task concurrency limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundTaskConfig {
    #[serde(default = "default_provider_concurrency")]
    pub provider_concurrency: usize,
    #[serde(default = "default_model_concurrency")]
    pub model_concurrency: usize,
}

fn default_provider_concurrency() -> usize {
    3
}

fn default_model_concurrency() -> usize {
    5
}

impl Default for BackgroundTaskConfig {
    fn default() -> Self {
        Self {
            provider_concurrency: 3,
            model_concurrency: 5,
        }
    }
}

/// Argus auto-review configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ArgusAutoReviewConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub threshold: Option<u32>,
    #[serde(default)]
    pub model: Option<String>,
}

/// MCP server definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct McpServerConfig {
    #[serde(rename = "type", default)]
    pub server_type: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// Experimental feature flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ExperimentalConfig {
    #[serde(default)]
    pub fork_agent_enabled: bool,
    #[serde(default)]
    pub hashline_edit: bool,
    #[serde(default)]
    pub model_fallback: bool,
    /// Enable the speculation engine: pre-run Write/Edit/MultiEdit/Bash
    /// tools inside an isolated `/tmp/jfc-speculation` overlay before the
    /// user approves them, so commit-on-approve feels instantaneous.
    /// Default off; opt-in until the engine has soaked in production.
    #[serde(default)]
    pub speculation_enabled: bool,
}

/// Feature configuration loaded from `.jfc/features.toml`.
///
/// Missing file → defaults. Malformed TOML → warning + defaults.
#[cfg(any(
    feature = "permission-automation",
    feature = "hooks",
    feature = "intent-gate",
    feature = "background-agents"
))]
pub mod feature_config {
    use serde::Deserialize;
    use std::path::Path;

    #[derive(Debug, Clone, Default, Deserialize)]
    #[serde(default)]
    pub struct FeatureConfig {
        pub permissions: PermissionsConfig,
        pub hooks: HooksConfig,
        pub intent: IntentConfig,
        pub background: BackgroundConfig,
    }

    #[derive(Debug, Clone, Deserialize)]
    #[serde(default)]
    pub struct PermissionsConfig {
        pub enabled: bool,
        pub rules: Vec<PermissionRuleConfig>,
        pub ceiling: Vec<String>,
    }

    impl Default for PermissionsConfig {
        fn default() -> Self {
            Self {
                enabled: false,
                rules: Vec::new(),
                ceiling: vec![
                    "Bash:rm -rf *".to_owned(),
                    "Bash:dd *".to_owned(),
                    "Bash:mkfs *".to_owned(),
                ],
            }
        }
    }

    #[derive(Debug, Clone, Default, Deserialize)]
    pub struct PermissionRuleConfig {
        pub action: String,
        pub tool: String,
        pub path: Option<String>,
        pub reason: Option<String>,
    }

    #[derive(Debug, Clone, Deserialize)]
    #[serde(default)]
    pub struct HooksConfig {
        pub enabled: bool,
        pub comment_check: CommentCheckConfig,
    }

    impl Default for HooksConfig {
        fn default() -> Self {
            Self {
                enabled: false,
                comment_check: CommentCheckConfig::default(),
            }
        }
    }

    #[derive(Debug, Clone, Deserialize)]
    #[serde(default)]
    pub struct CommentCheckConfig {
        pub enabled: bool,
        pub patterns: Vec<String>,
    }

    impl Default for CommentCheckConfig {
        fn default() -> Self {
            Self {
                enabled: false,
                patterns: vec![
                    "// This function".to_owned(),
                    "// TODO: implement".to_owned(),
                ],
            }
        }
    }

    #[derive(Debug, Clone, Deserialize)]
    #[serde(default)]
    pub struct IntentConfig {
        pub enabled: bool,
        pub confidence_threshold: f32,
    }

    impl Default for IntentConfig {
        fn default() -> Self {
            Self {
                enabled: false,
                confidence_threshold: 0.6,
            }
        }
    }

    #[derive(Debug, Clone, Deserialize)]
    #[serde(default)]
    pub struct BackgroundConfig {
        pub max_concurrent: usize,
        pub max_depth: usize,
    }

    impl Default for BackgroundConfig {
        fn default() -> Self {
            Self {
                max_concurrent: 5,
                max_depth: 2,
            }
        }
    }

    impl FeatureConfig {
        /// Load from `.jfc/features.toml` relative to `base_dir`.
        /// Returns defaults if file missing or malformed.
        pub fn load(base_dir: &Path) -> Self {
            let path = base_dir.join(".jfc").join("features.toml");
            match std::fs::read_to_string(&path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(config) => config,
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "malformed features.toml, using defaults"
                        );
                        Self::default()
                    }
                },
                Err(_) => Self::default(),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_feature_config_missing_file() {
            let tmp = tempfile::tempdir().unwrap();
            let config = FeatureConfig::load(tmp.path());
            assert!(!config.permissions.enabled);
            assert_eq!(config.background.max_concurrent, 5);
        }

        #[test]
        fn test_feature_config_valid_toml() {
            let tmp = tempfile::tempdir().unwrap();
            let jfc_dir = tmp.path().join(".jfc");
            std::fs::create_dir_all(&jfc_dir).unwrap();
            std::fs::write(
                jfc_dir.join("features.toml"),
                r#"
                [permissions]
                enabled = true

                [background]
                max_concurrent = 10
                "#,
            )
            .unwrap();
            let config = FeatureConfig::load(tmp.path());
            assert!(config.permissions.enabled);
            assert_eq!(config.background.max_concurrent, 10);
        }

        #[test]
        fn test_feature_config_malformed_toml() {
            let tmp = tempfile::tempdir().unwrap();
            let jfc_dir = tmp.path().join(".jfc");
            std::fs::create_dir_all(&jfc_dir).unwrap();
            std::fs::write(jfc_dir.join("features.toml"), "{{invalid toml").unwrap();
            let config = FeatureConfig::load(tmp.path());
            // Should return defaults without panicking
            assert!(!config.permissions.enabled);
        }
    }
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
    pub fallback_models: Vec<FallbackModel>,
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
    /// Per-tool permission overrides: { "Bash": "allow", "Edit": "deny", "Write": "ask" }
    #[serde(default)]
    pub permission: HashMap<String, String>,
    /// Text appended to the system prompt for this agent.
    #[serde(default)]
    pub prompt_append: Option<String>,
    /// Complete replacement system prompt (overrides default).
    #[serde(default)]
    pub prompt: Option<String>,
    /// OpenAI reasoning effort: "low", "medium", "high".
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// Sampling parameter (0.0 - 1.0).
    #[serde(default)]
    pub top_p: Option<f64>,
    /// Model variant (e.g. "max").
    #[serde(default)]
    pub variant: Option<String>,
    /// TUI color for this agent (hex like "#FF0000" or named like "blue").
    #[serde(default)]
    pub color: Option<String>,
    /// Provider-specific options passed through to the API.
    #[serde(default)]
    pub provider_options: HashMap<String, serde_json::Value>,
    /// Model to use for context compaction.
    #[serde(default)]
    pub compaction_model: Option<String>,
    /// Model to use for ultrawork mode.
    #[serde(default)]
    pub ultrawork_model: Option<String>,
    /// Text verbosity level: "concise", "normal", "verbose".
    #[serde(default)]
    pub text_verbosity: Option<String>,
}

/// A fallback model entry — either a plain string or an object with settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FallbackModel {
    /// Just a model ID string.
    Simple(String),
    /// Model with per-fallback settings.
    Detailed {
        model: String,
        #[serde(default)]
        variant: Option<String>,
        #[serde(default)]
        temperature: Option<f64>,
        #[serde(default)]
        reasoning_effort: Option<String>,
    },
}

impl FallbackModel {
    pub fn model_id(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Detailed { model, .. } => model,
        }
    }
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

/// Resolve a prompt value that may be a `file://` URI.
/// If it starts with `file://`, read the file contents.
/// Otherwise return the string as-is.
pub fn resolve_prompt(value: &str, base_dir: Option<&std::path::Path>) -> String {
    if let Some(path_str) = value.strip_prefix("file://") {
        let path = if let Some(base) = base_dir {
            base.join(path_str)
        } else {
            PathBuf::from(path_str)
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::config",
                    path = %path.display(),
                    error = %e,
                    "failed to read prompt file, using raw value"
                );
                value.to_owned()
            }
        }
    } else {
        value.to_owned()
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
                Some(m.model_id().to_owned())
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
        let cfg = parse(
            r#"
[default]
model = "x"
"#,
        );
        assert_eq!(cfg.default.model.as_deref(), Some("x"));
        assert!(cfg.agents.is_empty());
        assert!(cfg.default.fallback_models.is_empty());
        assert_eq!(cfg.default.temperature, None);
    }

    #[test]
    fn parse_full_config_normal() {
        let cfg = parse(
            r#"
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
"#,
        );
        // [default]
        assert_eq!(
            cfg.default.model.as_deref(),
            Some("anthropic/claude-opus-4-7")
        );
        assert_eq!(cfg.default.fallback_models[0].model_id(), "openai/gpt-5");
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
            reviewer.fallback_models[0].model_id(),
            "anthropic/claude-haiku-4-5"
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
        let cfg = parse(
            r#"
[default]
model = "B"

[agents.code-reviewer]
model = "A"
"#,
        );
        assert_eq!(
            resolve_model(&cfg, Some("code-reviewer")),
            Some("A".to_owned())
        );
    }

    #[test]
    fn resolve_model_falls_through_to_default_normal() {
        let cfg = parse(
            r#"
[default]
model = "B"

[agents.code-reviewer]
temperature = 0.1
"#,
        );
        assert_eq!(
            resolve_model(&cfg, Some("code-reviewer")),
            Some("B".to_owned())
        );
    }

    #[test]
    fn resolve_model_unknown_agent_uses_default_normal() {
        let cfg = parse(
            r#"
[default]
model = "B"
"#,
        );
        assert_eq!(
            resolve_model(&cfg, Some("does-not-exist")),
            Some("B".to_owned())
        );
    }

    #[test]
    fn resolve_model_uses_fallback_when_no_model_robust() {
        // Agent has no `model` field but a non-empty `fallback_models` list →
        // step 2 of the cascade returns the first fallback, NOT the default.
        let cfg = parse(
            r#"
[default]
model = "default-model"

[agents.formatter]
fallback_models = ["fallback-A", "fallback-B"]
"#,
        );
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
        let cfg = parse(
            r#"
[agents.code-reviewer]
disallowed_tools = ["Bash", "Write"]
"#,
        );
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
        let cfg2 = parse(
            r#"
[agents.formatter]
model = "x"
"#,
        );
        assert!(agent_disallowed(&cfg2, "formatter").is_empty());
    }

    #[test]
    fn parse_categories_normal() {
        let cfg = parse(
            r#"
[categories.visual-engineering]
model = "anthropic/claude-sonnet-4-6"
temperature = 0.3

[categories.writing]
model = "anthropic/claude-opus-4-7"
prompt_append = "Focus on clarity and conciseness."
"#,
        );
        assert_eq!(cfg.categories.len(), 2);
        let visual = cfg.categories.get("visual-engineering").unwrap();
        assert_eq!(visual.model.as_deref(), Some("anthropic/claude-sonnet-4-6"));
        assert_eq!(visual.temperature, Some(0.3));
    }

    #[test]
    fn parse_agent_permissions_normal() {
        let cfg = parse(
            r#"
[agents.reviewer]
model = "x"

[agents.reviewer.permission]
Bash = "deny"
Edit = "allow"
Write = "ask"
"#,
        );
        let reviewer = cfg.agents.get("reviewer").unwrap();
        assert_eq!(
            reviewer.permission.get("Bash").map(|s| s.as_str()),
            Some("deny")
        );
        assert_eq!(
            reviewer.permission.get("Edit").map(|s| s.as_str()),
            Some("allow")
        );
    }

    #[test]
    fn parse_prompt_append_normal() {
        let cfg = parse(
            r#"
[agents.coder]
model = "x"
prompt_append = "Always write tests."
"#,
        );
        let coder = cfg.agents.get("coder").unwrap();
        assert_eq!(coder.prompt_append.as_deref(), Some("Always write tests."));
    }

    #[test]
    fn parse_reasoning_effort_normal() {
        let cfg = parse(
            r#"
[agents.thinker]
model = "openai/o3"
reasoning_effort = "high"
top_p = 0.95
variant = "max"
"#,
        );
        let thinker = cfg.agents.get("thinker").unwrap();
        assert_eq!(thinker.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(thinker.top_p, Some(0.95));
        assert_eq!(thinker.variant.as_deref(), Some("max"));
    }

    #[test]
    fn parse_disabled_lists_normal() {
        let cfg = parse(
            r#"
disabled_agents = ["formatter", "linter"]
disabled_tools = ["Bash"]
"#,
        );
        assert_eq!(cfg.disabled_agents, vec!["formatter", "linter"]);
        assert_eq!(cfg.disabled_tools, vec!["Bash"]);
    }

    #[test]
    fn parse_mcp_config_normal() {
        let cfg = parse(
            r#"
[mcp.filesystem]
type = "stdio"
command = "npx"
args = ["-y", "@anthropic/mcp-filesystem"]

[mcp.filesystem.env]
HOME = "/home/user"
"#,
        );
        let fs = cfg.mcp.get("filesystem").unwrap();
        assert_eq!(fs.command.as_deref(), Some("npx"));
        assert_eq!(fs.args, vec!["-y", "@anthropic/mcp-filesystem"]);
        assert_eq!(fs.env.get("HOME").map(|s| s.as_str()), Some("/home/user"));
    }

    #[test]
    fn parse_fallback_models_mixed_normal() {
        let cfg = parse(
            r#"
[default]
model = "primary"
fallback_models = [
    "simple-fallback",
    { model = "detailed-fallback", variant = "max", temperature = 0.5 }
]
"#,
        );
        assert_eq!(cfg.default.fallback_models.len(), 2);
        assert_eq!(cfg.default.fallback_models[0].model_id(), "simple-fallback");
        assert_eq!(
            cfg.default.fallback_models[1].model_id(),
            "detailed-fallback"
        );
        if let FallbackModel::Detailed {
            variant,
            temperature,
            ..
        } = &cfg.default.fallback_models[1]
        {
            assert_eq!(variant.as_deref(), Some("max"));
            assert_eq!(*temperature, Some(0.5));
        } else {
            panic!("expected Detailed variant");
        }
    }

    #[test]
    fn parse_experimental_flags_normal() {
        let cfg = parse(
            r#"
[experimental]
hashline_edit = true
model_fallback = true
fork_agent_enabled = false
"#,
        );
        let exp = cfg.experimental.unwrap();
        assert!(exp.hashline_edit);
        assert!(exp.model_fallback);
        assert!(!exp.fork_agent_enabled);
    }

    #[test]
    fn parse_permission_automation_normal() {
        let cfg = parse(
            r#"
[permission_automation]
enabled = true

[[permission_automation.rules]]
action = "allow"
tool = "Edit"
path = "src/**"

[[permission_automation.rules]]
action = "deny"
tool = "Bash"
reason = "no shell access"
"#,
        );
        let pa = cfg.permission_automation.unwrap();
        assert!(pa.enabled);
        assert_eq!(pa.rules.len(), 2);
        assert_eq!(pa.rules[0].action, "allow");
        assert_eq!(pa.rules[1].tool, "Bash");
    }

    #[test]
    fn resolve_prompt_file_uri_normal() {
        let tmp = tempfile::tempdir().unwrap();
        let prompt_file = tmp.path().join("system.md");
        std::fs::write(&prompt_file, "You are a helpful assistant.").unwrap();
        let resolved = resolve_prompt("file://system.md", Some(tmp.path()));
        assert_eq!(resolved, "You are a helpful assistant.");
    }

    #[test]
    fn resolve_prompt_plain_string_normal() {
        let resolved = resolve_prompt("Just a plain prompt", None);
        assert_eq!(resolved, "Just a plain prompt");
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
