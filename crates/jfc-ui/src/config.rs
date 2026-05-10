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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// UI theme name (matches `Theme::by_name`). When omitted, jfc
    /// boots with the built-in dark theme. The `/theme <name>`
    /// command writes back to this field so the user's choice
    /// persists across restarts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Output style for assistant replies. One of: `default`,
    /// `brief`, `verbose`, `explanatory`, `learning`. Each style
    /// appends a suffix to the system prompt that nudges the model
    /// toward a different verbosity / scaffolding shape.
    /// `/output-style <name>` writes back to this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_style: Option<String>,
    /// Slate dynamic model routing toggle. Default `false` (legacy
    /// "use the pinned model for every turn" behavior). When `true`, each
    /// user turn passes through `slate::SlateRouter::route` before the
    /// stream call — see `crates/jfc-ui/src/slate.rs`.
    #[serde(default)]
    pub slate_enabled: bool,
    /// Per-`QueryClass` routing rules. Only consulted when `slate_enabled`.
    /// `None` = no rules, every classified query falls through to the
    /// pinned default model.
    #[serde(default)]
    pub slate_rules: Option<Vec<SlateRuleConfig>>,
    /// Enable two-phase memory recall (v132 `bt1`/`xt1`). When true (default)
    /// the stream-prep step issues two extra LLM calls per user turn — a
    /// "select" pass over memory filenames, then a "synthesize" pass over the
    /// chosen memories — and injects the synthesized facts as a
    /// `<system-reminder>` block. Worth ~150-600 extra prompt tokens for the
    /// recall calls; the savings come from injecting only the facts that
    /// matter into the main turn instead of every memory file.
    #[serde(default = "default_memory_recall_enabled")]
    pub memory_recall_enabled: bool,
    /// Optional per-session cost ceiling in USD. When set, the next-turn
    /// gate surfaces a warning toast at 80% and a high-priority warning
    /// at 100%; the user is never hard-blocked (a runaway estimate
    /// shouldn't kill an in-flight investigation), but they get a clear
    /// signal to call /quit or switch to a cheaper model.
    #[serde(default)]
    pub session_cost_budget_usd: Option<f64>,
}

fn default_memory_recall_enabled() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        // Hand-rolled rather than derive because `memory_recall_enabled`
        // defaults to `true` - `bool::default()` is `false` and the derived
        // impl would silently turn the recall pass off for every fresh
        // config. Every other field uses the derive-equivalent default.
        Self {
            default: AgentConfig::default(),
            agents: HashMap::new(),
            categories: HashMap::new(),
            permission_automation: None,
            background_task: None,
            argus_auto_review: None,
            mcp: HashMap::new(),
            disabled_agents: Vec::new(),
            disabled_tools: Vec::new(),
            experimental: None,
            theme: None,
            output_style: None,
            slate_enabled: false,
            slate_rules: None,
            memory_recall_enabled: default_memory_recall_enabled(),
            session_cost_budget_usd: None,
        }
    }
}

/// TOML form of a `slate::RoutingRule`. Lives here (not in `slate.rs`) so the
/// schema sits next to the rest of the config schema; the routing logic
/// converts these into `slate::RoutingRule` values at startup via
/// `slate_rules_from_config`. Splitting along this seam keeps `slate.rs`
/// independent of TOML / serde details and lets it stay testable in pure
/// Rust without a config fixture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlateRuleConfig {
    /// One of: `"trivial"`, `"exploration"`, `"code-edit"`, `"refactor"`,
    /// `"research"`, `"long-context"`. Unknown values are skipped at load
    /// time with a warning (no hard error — the user keeps a working
    /// router minus the bad rule).
    pub query_class: String,
    pub model: String,
    #[serde(default)]
    pub fallback_model: Option<String>,
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

/// Persist a chosen theme name to `~/.config/jfc/config.toml`,
/// preserving every other field the user has set. Returns the path
/// that was written, or an error string suitable for a toast.
///
/// We re-read the file (so we don't clobber concurrent edits the
/// user made by hand), update only the `theme` key, and serialize
/// back. When the file or its parent directory don't exist yet we
/// create them — first-time `/theme <name>` should Just Work without
/// the user having to `mkdir -p ~/.config/jfc/` themselves.
pub fn save_theme(theme_name: &str) -> Result<std::path::PathBuf, String> {
    save_theme_to(&config_path(), theme_name)
}

/// Test-friendly inner helper. Reads the file at `path` (treating
/// missing/empty as a fresh `Config::default()`), updates the theme
/// field, and writes the result back. Refuses to overwrite an
/// unparseable file so a user typo doesn't get silently clobbered.
pub fn save_theme_to(
    path: &std::path::Path,
    theme_name: &str,
) -> Result<std::path::PathBuf, String> {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            target: "jfc::config",
            path = %path.display(),
            error = %e,
            "save_theme: cannot create parent dir"
        );
        return Err(format!("cannot create {}: {e}", parent.display()));
    }
    let mut cfg: Config = match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => match toml::from_str(&s) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::config",
                    path = %path.display(),
                    error = %e,
                    "save_theme: refusing to overwrite unparseable config"
                );
                return Err(format!(
                    "{} is not valid TOML — fix it first ({e})",
                    path.display()
                ));
            }
        },
        _ => Config::default(),
    };
    cfg.theme = Some(theme_name.to_string());
    let serialized = toml::to_string_pretty(&cfg).map_err(|e| format!("serialize failed: {e}"))?;
    std::fs::write(path, serialized)
        .map_err(|e| format!("write {} failed: {e}", path.display()))?;
    tracing::info!(
        target: "jfc::config",
        path = %path.display(),
        theme = %theme_name,
        "save_theme: persisted theme"
    );
    Ok(path.to_path_buf())
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

/// Convert the TOML-form rules from a `Config` into the runtime
/// `slate::RoutingRule` values consumed by `SlateRouter`. Unknown
/// `query_class` values are dropped with a `tracing::warn!` so a typo doesn't
/// silently disable routing for a different class. Returns an empty `Vec`
/// when `slate_rules` is `None` or empty.
pub fn slate_rules_from_config(cfg: &Config) -> Vec<crate::slate::RoutingRule> {
    let Some(ref rules) = cfg.slate_rules else {
        return Vec::new();
    };
    rules
        .iter()
        .filter_map(|r| {
            match r.query_class.as_str() {
                "trivial" => Some(crate::slate::QueryClass::Trivial),
                "exploration" => Some(crate::slate::QueryClass::Exploration),
                "code-edit" => Some(crate::slate::QueryClass::CodeEdit),
                "refactor" => Some(crate::slate::QueryClass::Refactor),
                "research" => Some(crate::slate::QueryClass::Research),
                "long-context" => Some(crate::slate::QueryClass::LongContext),
                other => {
                    tracing::warn!(
                        target: "jfc::slate",
                        query_class = other,
                        "unknown slate query_class — rule dropped"
                    );
                    None
                }
            }
            .map(|class| {
                let mut rule = crate::slate::RoutingRule::new(class, r.model.clone());
                if let Some(ref fb) = r.fallback_model {
                    rule = rule.with_fallback(fb.clone());
                }
                rule
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Config {
        toml::from_str::<Config>(src).expect("expected valid toml")
    }

    // Normal: save_theme_to writes the field to a fresh file.
    #[test]
    fn save_theme_to_creates_new_file_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("nested").join("config.toml");
        save_theme_to(&path, "dracula").expect("write");
        assert!(path.exists(), "save_theme_to should create the file");
        let raw = std::fs::read_to_string(&path).expect("read");
        let parsed: Config = toml::from_str(&raw).expect("parse");
        assert_eq!(parsed.theme.as_deref(), Some("dracula"));
    }

    // Normal: save_theme_to preserves existing fields. The user's
    // model + agent block must survive a theme write — otherwise
    // `/theme dracula` would silently destroy their config.
    #[test]
    fn save_theme_to_preserves_other_fields_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[default]
model = "anthropic/claude-opus-4-7"

[agents.researcher]
model = "openai/gpt-5"
"#,
        )
        .unwrap();
        save_theme_to(&path, "tokyo-night").expect("write");
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.theme.as_deref(), Some("tokyo-night"));
        assert_eq!(
            cfg.default.model.as_deref(),
            Some("anthropic/claude-opus-4-7")
        );
        assert!(cfg.agents.contains_key("researcher"));
    }

    // Robust: a corrupted config must not get silently overwritten.
    // Returning an error gives the toast layer something to surface.
    #[test]
    fn save_theme_to_refuses_to_overwrite_broken_file_robust() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "this = is not [ valid toml").unwrap();
        let res = save_theme_to(&path, "dark");
        assert!(res.is_err(), "should refuse to overwrite invalid TOML");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("not [ valid"),
            "original contents must be preserved"
        );
    }

    // Normal: an empty file is treated as a fresh config — first-run
    // /theme should land in a clean file and parse afterwards.
    #[test]
    fn save_theme_to_treats_empty_file_as_fresh_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        save_theme_to(&path, "nord").expect("write");
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.theme.as_deref(), Some("nord"));
    }

    // Normal: theme field round-trips through serde.
    #[test]
    fn theme_field_roundtrips_normal() {
        let mut cfg = Config::default();
        cfg.theme = Some("monokai".into());
        let s = toml::to_string(&cfg).expect("serialize");
        assert!(s.contains("theme"));
        let back: Config = toml::from_str(&s).expect("parse");
        assert_eq!(back.theme.as_deref(), Some("monokai"));
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
    fn parse_slate_rules_normal() {
        let cfg = parse(
            r#"
slate_enabled = true

[[slate_rules]]
query_class = "trivial"
model = "claude-haiku-4-5"

[[slate_rules]]
query_class = "refactor"
model = "claude-opus-4-7"
fallback_model = "claude-sonnet-4-6"
"#,
        );
        assert!(cfg.slate_enabled);
        let rules = cfg.slate_rules.as_ref().expect("rules present");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].query_class, "trivial");
        assert_eq!(rules[0].model, "claude-haiku-4-5");
        assert!(rules[0].fallback_model.is_none());
        assert_eq!(
            rules[1].fallback_model.as_deref(),
            Some("claude-sonnet-4-6")
        );

        // Conversion path: TOML rules → runtime RoutingRule list.
        let rt = slate_rules_from_config(&cfg);
        assert_eq!(rt.len(), 2);
        assert_eq!(rt[0].query_class, crate::slate::QueryClass::Trivial);
        assert_eq!(rt[1].query_class, crate::slate::QueryClass::Refactor);
        assert_eq!(rt[1].fallback_model.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn parse_slate_unknown_class_dropped_robust() {
        let cfg = parse(
            r#"
slate_enabled = true

[[slate_rules]]
query_class = "trivial"
model = "haiku"

[[slate_rules]]
query_class = "not-a-real-class"
model = "garbage"
"#,
        );
        // Both rules parse at the TOML layer; the unknown one is filtered
        // out at the conversion layer.
        assert_eq!(cfg.slate_rules.as_ref().unwrap().len(), 2);
        let rt = slate_rules_from_config(&cfg);
        assert_eq!(rt.len(), 1);
        assert_eq!(rt[0].query_class, crate::slate::QueryClass::Trivial);
    }

    #[test]
    fn slate_disabled_by_default_robust() {
        let cfg = Config::default();
        assert!(!cfg.slate_enabled);
        assert!(cfg.slate_rules.is_none());
        assert!(slate_rules_from_config(&cfg).is_empty());
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
