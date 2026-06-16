use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    Config, FallbackModel, McpServerConfig, PermissionAutomationConfig, RemoteControlConfig,
    SandboxConfig, ShellHooksConfig, WorktreeConfig,
};

const CLAUDE_IN_CHROME_MCP_SERVER: &str = "claude-in-chrome";
const CLAUDE_IN_CHROME_MCP_ARG: &str = "--claude-in-chrome-mcp";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClaudeCompatibilityConfig {
    pub permissions: ClaudePermissionsConfig,
    pub env: HashMap<String, String>,
    pub model: Option<String>,
    #[serde(rename = "fallbackModel", alias = "fallback_model")]
    pub fallback_model: Option<String>,
    #[serde(rename = "availableModels", alias = "available_models")]
    pub available_models: Vec<String>,
    #[serde(rename = "modelOverrides", alias = "model_overrides")]
    pub model_overrides: HashMap<String, serde_json::Value>,
    pub agent: Option<String>,
    pub theme: Option<String>,
    #[serde(rename = "outputStyle", alias = "output_style")]
    pub output_style: Option<String>,
    pub verbose: Option<bool>,
    #[serde(rename = "defaultShell", alias = "default_shell")]
    pub default_shell: Option<String>,
    #[serde(rename = "alwaysThinkingEnabled", alias = "always_thinking_enabled")]
    pub always_thinking_enabled: Option<bool>,
    #[serde(rename = "thinkingBudget", alias = "thinking_budget")]
    pub thinking_budget: Option<u32>,
    #[serde(rename = "thinkingDisplay", alias = "thinking_display")]
    pub thinking_display: Option<String>,
    pub attribution: ClaudeAttributionConfig,
    pub worktree: Option<WorktreeConfig>,
    pub sandbox: Option<SandboxConfig>,
    #[serde(rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(
        rename = "enableAllProjectMcpServers",
        alias = "enable_all_project_mcp_servers"
    )]
    pub enable_all_project_mcp_servers: Option<bool>,
    #[serde(rename = "enabledMcpjsonServers", alias = "enabled_mcpjson_servers")]
    pub enabled_mcpjson_servers: Vec<String>,
    #[serde(rename = "disabledMcpjsonServers", alias = "disabled_mcpjson_servers")]
    pub disabled_mcpjson_servers: Vec<String>,
    #[serde(rename = "allowedMcpServers", alias = "allowed_mcp_servers")]
    pub allowed_mcp_servers: Vec<String>,
    #[serde(rename = "deniedMcpServers", alias = "denied_mcp_servers")]
    pub denied_mcp_servers: Vec<String>,
    #[serde(rename = "enabledPlugins", alias = "enabled_plugins")]
    pub enabled_plugins: HashMap<String, bool>,
    #[serde(
        rename = "claudeInChromeDefaultEnabled",
        alias = "claude_in_chrome_default_enabled"
    )]
    pub claude_in_chrome_default_enabled: Option<bool>,
    #[serde(rename = "chromeExtension", alias = "chrome_extension")]
    pub chrome_extension: Option<ClaudeChromeExtensionConfig>,
    #[serde(rename = "skillOverrides", alias = "skill_overrides")]
    pub skill_overrides: HashMap<String, serde_json::Value>,
    #[serde(default, deserialize_with = "deserialize_hooks")]
    pub hooks: Option<ShellHooksConfig>,
    #[serde(rename = "disableAllHooks", alias = "disable_all_hooks")]
    pub disable_all_hooks: Option<bool>,
    pub language: Option<String>,
    #[serde(rename = "cleanupPeriodDays", alias = "cleanup_period_days")]
    pub cleanup_period_days: Option<u32>,
    #[serde(rename = "respectGitignore", alias = "respect_gitignore")]
    pub respect_gitignore: Option<bool>,
    #[serde(rename = "spinnerTipsEnabled", alias = "spinner_tips_enabled")]
    pub spinner_tips_enabled: Option<bool>,
    #[serde(rename = "spinnerVerbs", alias = "spinner_verbs")]
    pub spinner_verbs: Option<serde_json::Value>,
    #[serde(rename = "spinnerTipsOverride", alias = "spinner_tips_override")]
    pub spinner_tips_override: Option<serde_json::Value>,
    #[serde(
        rename = "syntaxHighlightingDisabled",
        alias = "syntax_highlighting_disabled"
    )]
    pub syntax_highlighting_disabled: Option<bool>,
    #[serde(rename = "disableWorkflows", alias = "disable_workflows")]
    pub disable_workflows: Option<bool>,
    #[serde(rename = "enableWorkflows", alias = "enable_workflows")]
    pub enable_workflows: Option<bool>,
    #[serde(
        rename = "workflowKeywordTriggerEnabled",
        alias = "workflow_keyword_trigger_enabled"
    )]
    pub workflow_keyword_trigger_enabled: Option<bool>,
    #[serde(
        rename = "disableSkillShellExecution",
        alias = "disable_skill_shell_execution"
    )]
    pub disable_skill_shell_execution: Option<bool>,
    #[serde(rename = "disableAgentView", alias = "disable_agent_view")]
    pub disable_agent_view: Option<bool>,
    #[serde(rename = "disableRemoteControl", alias = "disable_remote_control")]
    pub disable_remote_control: Option<bool>,
    #[serde(rename = "includeCoAuthoredBy", alias = "include_co_authored_by")]
    pub include_co_authored_by: Option<bool>,
    #[serde(rename = "includeGitInstructions", alias = "include_git_instructions")]
    pub include_git_instructions: Option<bool>,
    #[serde(rename = "apiKeyHelper", alias = "api_key_helper")]
    pub api_key_helper: Option<String>,
    #[serde(rename = "proxyAuthHelper", alias = "proxy_auth_helper")]
    pub proxy_auth_helper: Option<String>,
    #[serde(rename = "statusLine", alias = "status_line")]
    pub status_line: Option<serde_json::Value>,
    #[serde(rename = "subagentStatusLine", alias = "subagent_status_line")]
    pub subagent_status_line: Option<serde_json::Value>,
    #[serde(rename = "parentSettingsBehavior", alias = "parent_settings_behavior")]
    pub parent_settings_behavior: Option<String>,

    // ── UX / display ────────────────────────────────────────────────────────
    #[serde(rename = "autoScrollEnabled", alias = "auto_scroll_enabled")]
    pub auto_scroll_enabled: Option<bool>,
    #[serde(rename = "showMessageTimestamps", alias = "show_message_timestamps")]
    pub show_message_timestamps: Option<bool>,
    #[serde(rename = "showTurnDuration", alias = "show_turn_duration")]
    pub show_turn_duration: Option<bool>,
    #[serde(rename = "showThinkingSummaries", alias = "show_thinking_summaries")]
    pub show_thinking_summaries: Option<bool>,
    #[serde(
        rename = "terminalProgressBarEnabled",
        alias = "terminal_progress_bar_enabled"
    )]
    pub terminal_progress_bar_enabled: Option<bool>,
    #[serde(
        rename = "terminalTitleFromRename",
        alias = "terminal_title_from_rename"
    )]
    pub terminal_title_from_rename: Option<bool>,
    #[serde(rename = "prefersReducedMotion", alias = "prefers_reduced_motion")]
    pub prefers_reduced_motion: Option<bool>,
    #[serde(rename = "hideVimModeIndicator", alias = "hide_vim_mode_indicator")]
    pub hide_vim_mode_indicator: Option<bool>,
    #[serde(
        rename = "promptSuggestionEnabled",
        alias = "prompt_suggestion_enabled"
    )]
    pub prompt_suggestion_enabled: Option<bool>,

    // ── Autonomous loop / memory ─────────────────────────────────────────────
    #[serde(rename = "autoDreamEnabled", alias = "auto_dream_enabled")]
    pub auto_dream_enabled: Option<bool>,
    #[serde(rename = "autoMemoryEnabled", alias = "auto_memory_enabled")]
    pub auto_memory_enabled: Option<bool>,
    #[serde(rename = "autoMemoryDirectory", alias = "auto_memory_directory")]
    pub auto_memory_directory: Option<String>,
    #[serde(rename = "awaySummaryEnabled", alias = "away_summary_enabled")]
    pub away_summary_enabled: Option<bool>,

    // ── Compaction ───────────────────────────────────────────────────────────
    #[serde(rename = "autoCompactEnabled", alias = "auto_compact_enabled")]
    pub auto_compact_enabled: Option<bool>,
    #[serde(rename = "autoCompactWindow", alias = "auto_compact_window")]
    pub auto_compact_window: Option<u32>,

    // ── Tasks / plans ────────────────────────────────────────────────────────
    #[serde(rename = "todoFeatureEnabled", alias = "todo_feature_enabled")]
    pub todo_feature_enabled: Option<bool>,
    #[serde(rename = "plansDirectory", alias = "plans_directory")]
    pub plans_directory: Option<String>,

    // ── Scheduling / breaks ──────────────────────────────────────────────────
    #[serde(rename = "quietHours", alias = "quiet_hours")]
    pub quiet_hours: Option<serde_json::Value>,
    #[serde(rename = "breakReminder", alias = "break_reminder")]
    pub break_reminder: Option<bool>,
    #[serde(rename = "breakThresholdMinutes", alias = "break_threshold_minutes")]
    pub break_threshold_minutes: Option<u32>,

    // ── Performance / mode ───────────────────────────────────────────────────
    #[serde(rename = "effortLevel", alias = "effort_level")]
    pub effort_level: Option<String>,
    #[serde(rename = "fastMode", alias = "fast_mode")]
    pub fast_mode: Option<bool>,
    #[serde(
        rename = "fastModePerSessionOptIn",
        alias = "fast_mode_per_session_opt_in"
    )]
    pub fast_mode_per_session_opt_in: Option<bool>,
    #[serde(rename = "bgIsolation", alias = "bg_isolation")]
    pub bg_isolation: Option<String>,
    #[serde(rename = "doneMeansMerged", alias = "done_means_merged")]
    pub done_means_merged: Option<bool>,

    // ── Channels / notifications ─────────────────────────────────────────────
    #[serde(rename = "channelsEnabled", alias = "channels_enabled")]
    pub channels_enabled: Option<bool>,
    #[serde(rename = "agentPushNotifEnabled", alias = "agent_push_notif_enabled")]
    pub agent_push_notif_enabled: Option<bool>,
    #[serde(
        rename = "inputNeededNotifEnabled",
        alias = "input_needed_notif_enabled"
    )]
    pub input_needed_notif_enabled: Option<bool>,
    #[serde(rename = "preferredNotifChannel", alias = "preferred_notif_channel")]
    pub preferred_notif_channel: Option<String>,

    // ── Misc ─────────────────────────────────────────────────────────────────
    pub voice: Option<serde_json::Value>,
    #[serde(rename = "sshConfigs", alias = "ssh_configs")]
    pub ssh_configs: Option<serde_json::Value>,
    #[serde(rename = "autoSubmit", alias = "auto_submit")]
    pub auto_submit: Option<bool>,
    #[serde(
        rename = "fileCheckpointingEnabled",
        alias = "file_checkpointing_enabled"
    )]
    pub file_checkpointing_enabled: Option<bool>,
    #[serde(rename = "teammateMode", alias = "teammate_mode")]
    pub teammate_mode: Option<String>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClaudeChromeExtensionConfig {
    #[serde(rename = "pairedDeviceId", alias = "paired_device_id")]
    pub paired_device_id: Option<String>,
}

impl ClaudeCompatibilityConfig {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    pub fn plugin_enabled(&self, plugin_name: &str) -> bool {
        let normalized = plugin_name.trim();
        if normalized.is_empty() {
            return true;
        }
        let mut explicit_enable = None;
        for (key, enabled) in &self.enabled_plugins {
            let key = key.trim();
            let matches = key == normalized
                || key
                    .split_once('@')
                    .map(|(name, _)| name == normalized)
                    .unwrap_or(false);
            if matches {
                if !*enabled {
                    return false;
                }
                explicit_enable = Some(true);
            }
        }
        explicit_enable.unwrap_or(true)
    }

    fn merge_from(&mut self, next: Self) {
        self.permissions.merge_from(next.permissions);
        merge_map(&mut self.env, next.env);
        overwrite_option(&mut self.model, next.model);
        overwrite_option(&mut self.fallback_model, next.fallback_model);
        extend_unique(&mut self.available_models, next.available_models);
        merge_map(&mut self.model_overrides, next.model_overrides);
        overwrite_option(&mut self.agent, next.agent);
        overwrite_option(&mut self.theme, next.theme);
        overwrite_option(&mut self.output_style, next.output_style);
        overwrite_option(&mut self.verbose, next.verbose);
        overwrite_option(&mut self.default_shell, next.default_shell);
        overwrite_option(
            &mut self.always_thinking_enabled,
            next.always_thinking_enabled,
        );
        overwrite_option(&mut self.thinking_budget, next.thinking_budget);
        overwrite_option(&mut self.thinking_display, next.thinking_display);
        self.attribution.merge_from(next.attribution);
        self.worktree = merge_worktree(self.worktree.take(), next.worktree);
        self.sandbox = merge_sandbox(self.sandbox.take(), next.sandbox);
        merge_map(&mut self.mcp_servers, next.mcp_servers);
        overwrite_option(
            &mut self.enable_all_project_mcp_servers,
            next.enable_all_project_mcp_servers,
        );
        extend_unique(
            &mut self.enabled_mcpjson_servers,
            next.enabled_mcpjson_servers,
        );
        extend_unique(
            &mut self.disabled_mcpjson_servers,
            next.disabled_mcpjson_servers,
        );
        extend_unique(&mut self.allowed_mcp_servers, next.allowed_mcp_servers);
        extend_unique(&mut self.denied_mcp_servers, next.denied_mcp_servers);
        merge_map(&mut self.enabled_plugins, next.enabled_plugins);
        overwrite_option(
            &mut self.claude_in_chrome_default_enabled,
            next.claude_in_chrome_default_enabled,
        );
        self.chrome_extension =
            merge_chrome_extension(self.chrome_extension.take(), next.chrome_extension);
        merge_map(&mut self.skill_overrides, next.skill_overrides);
        self.hooks = merge_hooks(self.hooks.take(), next.hooks);
        overwrite_option(&mut self.disable_all_hooks, next.disable_all_hooks);
        overwrite_option(&mut self.language, next.language);
        overwrite_option(&mut self.cleanup_period_days, next.cleanup_period_days);
        overwrite_option(&mut self.respect_gitignore, next.respect_gitignore);
        overwrite_option(&mut self.spinner_tips_enabled, next.spinner_tips_enabled);
        overwrite_option(&mut self.spinner_verbs, next.spinner_verbs);
        overwrite_option(&mut self.spinner_tips_override, next.spinner_tips_override);
        overwrite_option(
            &mut self.syntax_highlighting_disabled,
            next.syntax_highlighting_disabled,
        );
        overwrite_option(&mut self.disable_workflows, next.disable_workflows);
        overwrite_option(&mut self.enable_workflows, next.enable_workflows);
        overwrite_option(
            &mut self.workflow_keyword_trigger_enabled,
            next.workflow_keyword_trigger_enabled,
        );
        overwrite_option(
            &mut self.disable_skill_shell_execution,
            next.disable_skill_shell_execution,
        );
        overwrite_option(&mut self.disable_agent_view, next.disable_agent_view);
        overwrite_option(
            &mut self.disable_remote_control,
            next.disable_remote_control,
        );
        overwrite_option(
            &mut self.include_co_authored_by,
            next.include_co_authored_by,
        );
        overwrite_option(
            &mut self.include_git_instructions,
            next.include_git_instructions,
        );
        overwrite_option(&mut self.api_key_helper, next.api_key_helper);
        overwrite_option(&mut self.proxy_auth_helper, next.proxy_auth_helper);
        overwrite_option(&mut self.status_line, next.status_line);
        overwrite_option(&mut self.subagent_status_line, next.subagent_status_line);
        overwrite_option(
            &mut self.parent_settings_behavior,
            next.parent_settings_behavior,
        );

        // UX / display
        overwrite_option(&mut self.auto_scroll_enabled, next.auto_scroll_enabled);
        overwrite_option(
            &mut self.show_message_timestamps,
            next.show_message_timestamps,
        );
        overwrite_option(&mut self.show_turn_duration, next.show_turn_duration);
        overwrite_option(
            &mut self.show_thinking_summaries,
            next.show_thinking_summaries,
        );
        overwrite_option(
            &mut self.terminal_progress_bar_enabled,
            next.terminal_progress_bar_enabled,
        );
        overwrite_option(
            &mut self.terminal_title_from_rename,
            next.terminal_title_from_rename,
        );
        overwrite_option(
            &mut self.prefers_reduced_motion,
            next.prefers_reduced_motion,
        );
        overwrite_option(
            &mut self.hide_vim_mode_indicator,
            next.hide_vim_mode_indicator,
        );
        overwrite_option(
            &mut self.prompt_suggestion_enabled,
            next.prompt_suggestion_enabled,
        );

        // Autonomous loop / memory
        overwrite_option(&mut self.auto_dream_enabled, next.auto_dream_enabled);
        overwrite_option(&mut self.auto_memory_enabled, next.auto_memory_enabled);
        overwrite_option(&mut self.auto_memory_directory, next.auto_memory_directory);
        overwrite_option(&mut self.away_summary_enabled, next.away_summary_enabled);

        // Compaction
        overwrite_option(&mut self.auto_compact_enabled, next.auto_compact_enabled);
        overwrite_option(&mut self.auto_compact_window, next.auto_compact_window);

        // Tasks / plans
        overwrite_option(&mut self.todo_feature_enabled, next.todo_feature_enabled);
        overwrite_option(&mut self.plans_directory, next.plans_directory);

        // Scheduling / breaks
        overwrite_option(&mut self.quiet_hours, next.quiet_hours);
        overwrite_option(&mut self.break_reminder, next.break_reminder);
        overwrite_option(
            &mut self.break_threshold_minutes,
            next.break_threshold_minutes,
        );

        // Performance / mode
        overwrite_option(&mut self.effort_level, next.effort_level);
        overwrite_option(&mut self.fast_mode, next.fast_mode);
        overwrite_option(
            &mut self.fast_mode_per_session_opt_in,
            next.fast_mode_per_session_opt_in,
        );
        overwrite_option(&mut self.bg_isolation, next.bg_isolation);
        overwrite_option(&mut self.done_means_merged, next.done_means_merged);

        // Channels / notifications
        overwrite_option(&mut self.channels_enabled, next.channels_enabled);
        overwrite_option(
            &mut self.agent_push_notif_enabled,
            next.agent_push_notif_enabled,
        );
        overwrite_option(
            &mut self.input_needed_notif_enabled,
            next.input_needed_notif_enabled,
        );
        overwrite_option(
            &mut self.preferred_notif_channel,
            next.preferred_notif_channel,
        );

        // Misc
        overwrite_option(&mut self.voice, next.voice);
        overwrite_option(&mut self.ssh_configs, next.ssh_configs);
        overwrite_option(&mut self.auto_submit, next.auto_submit);
        overwrite_option(
            &mut self.file_checkpointing_enabled,
            next.file_checkpointing_enabled,
        );
        overwrite_option(&mut self.teammate_mode, next.teammate_mode);

        merge_map(&mut self.extra, next.extra);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClaudePermissionsConfig {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub ask: Vec<String>,
    #[serde(rename = "defaultMode", alias = "default_mode")]
    pub default_mode: Option<String>,
    #[serde(rename = "additionalDirectories", alias = "additional_directories")]
    pub additional_directories: Vec<String>,
}

impl ClaudePermissionsConfig {
    fn merge_from(&mut self, next: Self) {
        extend_unique(&mut self.allow, next.allow);
        extend_unique(&mut self.deny, next.deny);
        extend_unique(&mut self.ask, next.ask);
        overwrite_option(&mut self.default_mode, next.default_mode);
        extend_unique(
            &mut self.additional_directories,
            next.additional_directories,
        );
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClaudeAttributionConfig {
    pub commit: Option<String>,
    pub pr: Option<String>,
}

impl ClaudeAttributionConfig {
    fn merge_from(&mut self, next: Self) {
        overwrite_option(&mut self.commit, next.commit);
        overwrite_option(&mut self.pr, next.pr);
    }
}

pub fn settings_paths(project_root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".claude").join("settings.json"));
    }
    paths.push(project_root.join(".claude").join("settings.json"));
    paths.push(project_root.join(".claude").join("settings.local.json"));
    paths
}

pub fn load_merged(project_root: &Path) -> ClaudeCompatibilityConfig {
    let mut merged = ClaudeCompatibilityConfig::default();
    for path in settings_paths(project_root) {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<ClaudeCompatibilityConfig>(&raw) {
            Ok(settings) => merged.merge_from(settings),
            Err(error) => {
                tracing::warn!(
                    target: "jfc::config::claude_settings",
                    path = %path.display(),
                    error = %error,
                    "failed to parse Claude settings JSON - ignoring"
                );
            }
        }
    }
    merged
}

pub fn apply_to_config(cfg: &mut Config, project_root: &Path) {
    let settings = load_merged(project_root);
    apply_settings(cfg, settings);
}

pub fn apply_settings(cfg: &mut Config, settings: ClaudeCompatibilityConfig) {
    if settings.is_empty() && !chrome_mcp_auto_enable_enabled(&settings) {
        return;
    }

    if let Some(model) = settings
        .model
        .as_ref()
        .filter(|model| !model.trim().is_empty())
    {
        cfg.default.model = Some(model.trim().to_owned());
    }
    if let Some(fallback_model) = settings
        .fallback_model
        .as_ref()
        .filter(|model| !model.trim().is_empty())
    {
        let fallback = FallbackModel::Simple(fallback_model.trim().to_owned());
        if !cfg
            .default
            .fallback_models
            .iter()
            .any(|existing| existing.model_id() == fallback_model.trim())
        {
            cfg.default.fallback_models.push(fallback);
        }
        cfg.refusal_fallback_model = Some(fallback_model.trim().to_owned());
    }
    if let Some(theme) = settings
        .theme
        .as_ref()
        .filter(|theme| !theme.trim().is_empty())
    {
        cfg.theme = Some(theme.trim().to_owned());
    }
    if let Some(output_style) = settings
        .output_style
        .as_ref()
        .filter(|style| !style.trim().is_empty())
    {
        cfg.output_style = Some(output_style.trim().to_owned());
    }
    if let Some(shell) = settings
        .default_shell
        .as_ref()
        .filter(|shell| !shell.trim().is_empty())
    {
        cfg.default_shell = Some(shell.trim().to_owned());
    }
    if let Some(always_thinking) = settings.always_thinking_enabled {
        cfg.default.provider_options.insert(
            "alwaysThinkingEnabled".to_owned(),
            serde_json::Value::Bool(always_thinking),
        );
    }
    if let Some(thinking_budget) = settings.thinking_budget {
        cfg.default.thinking_budget = Some(thinking_budget);
    }
    if let Some(thinking_display) = settings
        .thinking_display
        .as_ref()
        .filter(|display| !display.trim().is_empty())
    {
        cfg.default.provider_options.insert(
            "thinkingDisplay".to_owned(),
            serde_json::Value::String(thinking_display.trim().to_owned()),
        );
    }
    if let Some(mode) = settings
        .permissions
        .default_mode
        .as_deref()
        .and_then(normalize_permission_mode)
    {
        cfg.default
            .permission
            .insert("mode".to_owned(), mode.to_owned());
    }
    if !settings.permissions.allow.is_empty() || !settings.permissions.deny.is_empty() {
        let automation = cfg
            .permission_automation
            .get_or_insert_with(PermissionAutomationConfig::default);
        automation.enabled = true;
        extend_unique(
            &mut automation.allowed_tools,
            settings.permissions.allow.clone(),
        );
        extend_unique(
            &mut automation.denied_tools,
            settings.permissions.deny.clone(),
        );
    }
    if let Some(hooks) = settings.hooks.clone() {
        cfg.hooks = merge_hooks(cfg.hooks.take(), Some(hooks));
    }
    if settings.disable_all_hooks == Some(true) {
        cfg.hooks = None;
    }
    for (name, server) in &settings.mcp_servers {
        cfg.mcp.insert(name.clone(), server.clone());
    }
    apply_chrome_mcp_auto_enable(cfg, &settings);
    if let Some(worktree) = settings.worktree.clone() {
        cfg.worktree = merge_worktree(cfg.worktree.take(), Some(worktree));
    }
    if let Some(sandbox) = settings.sandbox.clone() {
        cfg.sandbox = merge_sandbox(cfg.sandbox.take(), Some(sandbox));
    }
    if settings.disable_remote_control == Some(true) {
        cfg.remote_control
            .get_or_insert_with(RemoteControlConfig::default)
            .disabled = true;
    }
    if settings.disable_agent_view == Some(true) {
        cfg.disabled_tools.push("AgentView".to_owned());
    }
    if settings.disable_workflows == Some(true) && settings.enable_workflows != Some(true) {
        cfg.disabled_tools.push("Workflow".to_owned());
    }

    // Compaction settings — bridge into Config's own fields
    if let Some(enabled) = settings.auto_compact_enabled {
        cfg.auto_compact_enabled = enabled;
    }
    if let Some(window) = settings.auto_compact_window {
        cfg.auto_compact_window = Some(window);
    }

    // Memory recall — autoMemoryEnabled gates memory_recall_enabled.
    if let Some(enabled) = settings.auto_memory_enabled {
        cfg.memory_recall_enabled = enabled;
    }

    // Effort level → reasoning_effort + thinking budget mapping.
    // CC 2.1.167 maps effortLevel to the model's reasoning effort level.
    if let Some(ref level) = settings.effort_level {
        let effort_str = match level.trim() {
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "max" | "xhigh" => Some("max"),
            _ => None,
        };
        if let Some(e) = effort_str {
            if cfg.default.reasoning_effort.is_none() {
                cfg.default.reasoning_effort = Some(e.to_owned());
            }
        }
        // Also set thinking budget as a fallback for providers that use it.
        let budget = match level.trim() {
            "low" => Some(1_000u32),
            "medium" => Some(5_000u32),
            "high" => Some(10_000u32),
            "max" | "xhigh" => Some(32_000u32),
            _ => None,
        };
        if let Some(b) = budget {
            cfg.default.thinking_budget = cfg.default.thinking_budget.or(Some(b));
        }
        // "max"/"xhigh" → ultracode
        if matches!(level.trim(), "max" | "xhigh") && cfg.default.ultracode.is_none() {
            cfg.default.ultracode = Some(true);
        }
    }

    cfg.claude = settings;
}

fn normalize_permission_mode(mode: &str) -> Option<&'static str> {
    match mode.trim() {
        "default" => Some("default"),
        "plan" => Some("plan"),
        "acceptEdits" | "accept-edits" | "accept_edits" => Some("accept-edits"),
        "dontAsk" | "dont-ask" | "bypass" | "bypassPermissions" | "bypass-permissions" => {
            Some("bypass")
        }
        "auto" => Some("auto"),
        _ => None,
    }
}

fn apply_chrome_mcp_auto_enable(cfg: &mut Config, settings: &ClaudeCompatibilityConfig) {
    if !chrome_mcp_auto_enable_enabled(settings) {
        return;
    }
    if cfg.mcp.contains_key(CLAUDE_IN_CHROME_MCP_SERVER) {
        return;
    }
    if !mcp_server_allowed_by_settings(settings, CLAUDE_IN_CHROME_MCP_SERVER) {
        tracing::info!(
            target: "jfc::config::claude_settings",
            server = CLAUDE_IN_CHROME_MCP_SERVER,
            "skipping Claude in Chrome MCP auto-enable because settings deny it"
        );
        return;
    }

    cfg.mcp.insert(
        CLAUDE_IN_CHROME_MCP_SERVER.to_owned(),
        McpServerConfig {
            server_type: Some("stdio".to_owned()),
            command: Some(chrome_mcp_command()),
            args: chrome_mcp_args(),
            env: HashMap::new(),
            env_file: None,
            headers: HashMap::new(),
            url: None,
        },
    );
}

fn chrome_mcp_auto_enable_enabled(settings: &ClaudeCompatibilityConfig) -> bool {
    if env_truthy("JFC_DISABLE_CHROME")
        || env_truthy("JFC_NO_CHROME")
        || env_truthy("CLAUDE_CODE_DISABLE_CFC")
    {
        return false;
    }
    if env_truthy("JFC_CHROME")
        || env_truthy("JFC_ENABLE_CHROME")
        || env_truthy("CLAUDE_CODE_ENABLE_CFC")
    {
        return true;
    }
    settings.claude_in_chrome_default_enabled == Some(true)
}

#[cfg(test)]
fn chrome_mcp_command() -> String {
    "claude".to_owned()
}

#[cfg(not(test))]
fn chrome_mcp_command() -> String {
    std::env::var("JFC_CHROME_MCP_COMMAND")
        .ok()
        .or_else(|| std::env::var("CLAUDE_CODE_CHROME_MCP_COMMAND").ok())
        .filter(|command| !command.trim().is_empty())
        .unwrap_or_else(|| "claude".to_owned())
}

#[cfg(test)]
fn chrome_mcp_args() -> Vec<String> {
    vec![CLAUDE_IN_CHROME_MCP_ARG.to_owned()]
}

#[cfg(not(test))]
fn chrome_mcp_args() -> Vec<String> {
    let raw = std::env::var("JFC_CHROME_MCP_ARGS")
        .ok()
        .or_else(|| std::env::var("CLAUDE_CODE_CHROME_MCP_ARGS").ok());
    raw.map(|args| {
        args.split_whitespace()
            .filter(|arg| !arg.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    })
    .filter(|args| !args.is_empty())
    .unwrap_or_else(|| vec![CLAUDE_IN_CHROME_MCP_ARG.to_owned()])
}

fn mcp_server_allowed_by_settings(settings: &ClaudeCompatibilityConfig, server: &str) -> bool {
    if string_list_contains(&settings.denied_mcp_servers, server) {
        return false;
    }
    settings.allowed_mcp_servers.is_empty()
        || string_list_contains(&settings.allowed_mcp_servers, server)
}

fn string_list_contains(values: &[String], needle: &str) -> bool {
    values
        .iter()
        .any(|value| value.trim().eq_ignore_ascii_case(needle))
}

#[cfg(test)]
fn env_truthy(_key: &str) -> bool {
    false
}

#[cfg(not(test))]
fn env_truthy(key: &str) -> bool {
    std::env::var(key).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn merge_hooks(
    current: Option<ShellHooksConfig>,
    next: Option<ShellHooksConfig>,
) -> Option<ShellHooksConfig> {
    match (current, next) {
        (None, None) => None,
        (Some(hooks), None) | (None, Some(hooks)) => Some(hooks),
        (Some(mut current), Some(mut next)) => {
            current.pre_tool_use.append(&mut next.pre_tool_use);
            current.post_tool_use.append(&mut next.post_tool_use);
            current
                .post_tool_use_failure
                .append(&mut next.post_tool_use_failure);
            current
                .user_prompt_submit
                .append(&mut next.user_prompt_submit);
            current.session_start.append(&mut next.session_start);
            current.session_end.append(&mut next.session_end);
            current.stop.append(&mut next.stop);
            current.subagent_stop.append(&mut next.subagent_stop);
            current.setup.append(&mut next.setup);
            current
                .user_prompt_expansion
                .append(&mut next.user_prompt_expansion);
            current.message_display.append(&mut next.message_display);
            current.elicitation.append(&mut next.elicitation);
            current
                .elicitation_result
                .append(&mut next.elicitation_result);
            current.post_tool_batch.append(&mut next.post_tool_batch);
            current.post_compact.append(&mut next.post_compact);
            current.subagent_start.append(&mut next.subagent_start);
            current
                .permission_request
                .append(&mut next.permission_request);
            current
                .permission_denied
                .append(&mut next.permission_denied);
            current.task_created.append(&mut next.task_created);
            current.task_completed.append(&mut next.task_completed);
            current.worktree_create.append(&mut next.worktree_create);
            current.worktree_remove.append(&mut next.worktree_remove);
            current.config_change.append(&mut next.config_change);
            current
                .instructions_loaded
                .append(&mut next.instructions_loaded);
            current.cwd_changed.append(&mut next.cwd_changed);
            current.file_changed.append(&mut next.file_changed);
            current.teammate_idle.append(&mut next.teammate_idle);
            current.stop_failure.append(&mut next.stop_failure);
            Some(current)
        }
    }
}

fn merge_chrome_extension(
    current: Option<ClaudeChromeExtensionConfig>,
    next: Option<ClaudeChromeExtensionConfig>,
) -> Option<ClaudeChromeExtensionConfig> {
    match (current, next) {
        (None, None) => None,
        (Some(extension), None) | (None, Some(extension)) => Some(extension),
        (Some(mut current), Some(next)) => {
            overwrite_option(&mut current.paired_device_id, next.paired_device_id);
            Some(current)
        }
    }
}

fn merge_worktree(
    current: Option<WorktreeConfig>,
    next: Option<WorktreeConfig>,
) -> Option<WorktreeConfig> {
    match (current, next) {
        (None, None) => None,
        (Some(worktree), None) | (None, Some(worktree)) => Some(worktree),
        (Some(mut current), Some(next)) => {
            overwrite_option(&mut current.base_ref, next.base_ref);
            extend_unique(&mut current.sparse_paths, next.sparse_paths);
            extend_unique(&mut current.symlink_directories, next.symlink_directories);
            Some(current)
        }
    }
}

fn merge_sandbox(
    current: Option<SandboxConfig>,
    next: Option<SandboxConfig>,
) -> Option<SandboxConfig> {
    match (current, next) {
        (None, None) => None,
        (Some(sandbox), None) | (None, Some(sandbox)) => Some(sandbox),
        (Some(mut current), Some(next)) => {
            overwrite_option(&mut current.enabled, next.enabled);
            overwrite_option(&mut current.fail_if_unavailable, next.fail_if_unavailable);
            overwrite_option(
                &mut current.auto_allow_bash_if_sandboxed,
                next.auto_allow_bash_if_sandboxed,
            );
            extend_unique(
                &mut current.allow_unsandboxed_commands,
                next.allow_unsandboxed_commands,
            );
            overwrite_option(&mut current.ignore_violations, next.ignore_violations);
            overwrite_option(
                &mut current.enable_weaker_nested_sandbox,
                next.enable_weaker_nested_sandbox,
            );
            overwrite_option(
                &mut current.enable_weaker_network_isolation,
                next.enable_weaker_network_isolation,
            );
            extend_unique(&mut current.excluded_commands, next.excluded_commands);
            overwrite_option(&mut current.bwrap_path, next.bwrap_path);
            overwrite_option(&mut current.socat_path, next.socat_path);

            extend_unique(
                &mut current.network.allowed_domains,
                next.network.allowed_domains,
            );
            extend_unique(
                &mut current.network.denied_domains,
                next.network.denied_domains,
            );
            overwrite_option(
                &mut current.network.allow_managed_domains_only,
                next.network.allow_managed_domains_only,
            );
            overwrite_option(
                &mut current.network.allow_unix_sockets,
                next.network.allow_unix_sockets,
            );
            overwrite_option(
                &mut current.network.allow_local_binding,
                next.network.allow_local_binding,
            );
            overwrite_option(
                &mut current.network.http_proxy_port,
                next.network.http_proxy_port,
            );
            overwrite_option(
                &mut current.network.socks_proxy_port,
                next.network.socks_proxy_port,
            );
            overwrite_option(
                &mut current.network.tls_termination,
                next.network.tls_termination,
            );

            extend_unique(
                &mut current.filesystem.allow_write,
                next.filesystem.allow_write,
            );
            extend_unique(
                &mut current.filesystem.deny_write,
                next.filesystem.deny_write,
            );
            extend_unique(
                &mut current.filesystem.allow_read,
                next.filesystem.allow_read,
            );
            extend_unique(&mut current.filesystem.deny_read, next.filesystem.deny_read);
            overwrite_option(
                &mut current.filesystem.allow_managed_read_paths_only,
                next.filesystem.allow_managed_read_paths_only,
            );

            overwrite_option(&mut current.ripgrep.command, next.ripgrep.command);
            extend_unique(&mut current.ripgrep.args, next.ripgrep.args);
            Some(current)
        }
    }
}

fn deserialize_hooks<'de, D>(deserializer: D) -> Result<Option<ShellHooksConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Some(value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Ok(flat) = serde_json::from_value::<ShellHooksConfig>(value.clone()) {
        return Ok(Some(flat));
    }
    parse_claude_hook_value(value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

fn parse_claude_hook_value(value: serde_json::Value) -> Result<ShellHooksConfig, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "hooks must be an object".to_owned())?;
    let mut out = ShellHooksConfig::default();
    for (event, entries) in object {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let matcher = entry
                .get("matcher")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            let inherited_async = entry
                .get("async")
                .or_else(|| entry.get("async_mode"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false);

            if let Some(command) = entry.get("command").and_then(|value| value.as_str()) {
                push_hook(
                    &mut out,
                    event,
                    crate::ShellHookEntry {
                        matcher: matcher.clone(),
                        command: command.to_owned(),
                        async_mode: inherited_async,
                    },
                );
            }

            let Some(hooks) = entry.get("hooks").and_then(|value| value.as_array()) else {
                continue;
            };
            for hook in hooks {
                let hook_type = hook
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or("command");
                if hook_type != "command" {
                    continue;
                }
                let Some(command) = hook.get("command").and_then(|value| value.as_str()) else {
                    continue;
                };
                let async_mode = hook
                    .get("async")
                    .or_else(|| hook.get("async_mode"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(inherited_async);
                push_hook(
                    &mut out,
                    event,
                    crate::ShellHookEntry {
                        matcher: matcher.clone(),
                        command: command.to_owned(),
                        async_mode,
                    },
                );
            }
        }
    }
    Ok(out)
}

fn push_hook(out: &mut ShellHooksConfig, event: &str, entry: crate::ShellHookEntry) {
    match event {
        "PreToolUse" | "preToolUse" | "pre_tool_use" => out.pre_tool_use.push(entry),
        "PostToolUse" | "postToolUse" | "post_tool_use" => out.post_tool_use.push(entry),
        "PostToolUseFailure" | "postToolUseFailure" | "post_tool_use_failure" => {
            out.post_tool_use_failure.push(entry)
        }
        "UserPromptSubmit" | "userPromptSubmit" | "user_prompt_submit" => {
            out.user_prompt_submit.push(entry)
        }
        "SessionStart" | "sessionStart" | "session_start" => out.session_start.push(entry),
        "SessionEnd" | "sessionEnd" | "session_end" => out.session_end.push(entry),
        "Stop" | "stop" => out.stop.push(entry),
        "SubagentStop" | "subagentStop" | "subagent_stop" => out.subagent_stop.push(entry),
        "Setup" | "setup" => out.setup.push(entry),
        "UserPromptExpansion" | "userPromptExpansion" | "user_prompt_expansion" => {
            out.user_prompt_expansion.push(entry)
        }
        "MessageDisplay" | "messageDisplay" | "message_display" => out.message_display.push(entry),
        "Elicitation" | "elicitation" => out.elicitation.push(entry),
        "ElicitationResult" | "elicitationResult" | "elicitation_result" => {
            out.elicitation_result.push(entry)
        }
        "PostToolBatch" | "postToolBatch" | "post_tool_batch" => out.post_tool_batch.push(entry),
        "PostCompact" | "postCompact" | "post_compact" => out.post_compact.push(entry),
        "SubagentStart" | "subagentStart" | "subagent_start" => out.subagent_start.push(entry),
        "PermissionRequest" | "permissionRequest" | "permission_request" => {
            out.permission_request.push(entry)
        }
        "PermissionDenied" | "permissionDenied" | "permission_denied" => {
            out.permission_denied.push(entry)
        }
        "TaskCreated" | "taskCreated" | "task_created" => out.task_created.push(entry),
        "TaskCompleted" | "taskCompleted" | "task_completed" => out.task_completed.push(entry),
        "WorktreeCreate" | "worktreeCreate" | "worktree_create" => out.worktree_create.push(entry),
        "WorktreeRemove" | "worktreeRemove" | "worktree_remove" => out.worktree_remove.push(entry),
        "ConfigChange" | "configChange" | "config_change" => out.config_change.push(entry),
        "InstructionsLoaded" | "instructionsLoaded" | "instructions_loaded" => {
            out.instructions_loaded.push(entry)
        }
        "CwdChanged" | "cwdChanged" | "cwd_changed" => out.cwd_changed.push(entry),
        "FileChanged" | "fileChanged" | "file_changed" => out.file_changed.push(entry),
        "TeammateIdle" | "teammateIdle" | "teammate_idle" => out.teammate_idle.push(entry),
        "StopFailure" | "stopFailure" | "stop_failure" => out.stop_failure.push(entry),
        "UserInterrupt" | "userInterrupt" | "user_interrupt" => out.user_interrupt.push(entry),
        "ModelResponseChunk" | "modelResponseChunk" | "model_response_chunk" => {
            out.model_response_chunk.push(entry)
        }
        "UserInputRequired" | "userInputRequired" | "user_input_required" => {
            out.user_input_required.push(entry)
        }
        _ => {}
    }
}

fn overwrite_option<T>(slot: &mut Option<T>, next: Option<T>) {
    if next.is_some() {
        *slot = next;
    }
}

fn merge_map<K, V>(slot: &mut HashMap<K, V>, next: HashMap<K, V>)
where
    K: std::hash::Hash + Eq,
{
    slot.extend(next);
}

fn extend_unique(slot: &mut Vec<String>, next: Vec<String>) {
    for value in next {
        if !slot.iter().any(|existing| existing == &value) {
            slot.push(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_layers_permissions_and_plugins_normal() {
        let mut base = ClaudeCompatibilityConfig {
            permissions: ClaudePermissionsConfig {
                allow: vec!["Read".to_owned()],
                default_mode: Some("plan".to_owned()),
                ..Default::default()
            },
            enabled_plugins: HashMap::from([("formatter@source".to_owned(), true)]),
            ..Default::default()
        };
        base.merge_from(ClaudeCompatibilityConfig {
            permissions: ClaudePermissionsConfig {
                allow: vec!["Bash(cargo test *)".to_owned()],
                deny: vec!["Bash(rm -rf *)".to_owned()],
                default_mode: Some("acceptEdits".to_owned()),
                additional_directories: vec!["/tmp/shared".to_owned()],
                ..Default::default()
            },
            enabled_plugins: HashMap::from([("formatter@source".to_owned(), false)]),
            model: Some("sonnet".to_owned()),
            ..Default::default()
        });

        assert_eq!(base.model.as_deref(), Some("sonnet"));
        assert_eq!(
            base.permissions.default_mode.as_deref(),
            Some("acceptEdits")
        );
        assert!(base.permissions.allow.iter().any(|item| item == "Read"));
        assert!(
            base.permissions
                .allow
                .iter()
                .any(|item| item == "Bash(cargo test *)")
        );
        assert!(!base.plugin_enabled("formatter"));
    }

    #[test]
    fn apply_settings_maps_supported_keys_into_config_normal() {
        let mut cfg = Config::default();
        apply_settings(
            &mut cfg,
            ClaudeCompatibilityConfig {
                model: Some("opus".to_owned()),
                permissions: ClaudePermissionsConfig {
                    allow: vec!["Read".to_owned()],
                    deny: vec!["Bash(rm -rf *)".to_owned()],
                    ask: vec!["Write(/etc/*)".to_owned()],
                    default_mode: Some("dontAsk".to_owned()),
                    additional_directories: vec!["/extra".to_owned()],
                },
                ..Default::default()
            },
        );

        assert_eq!(cfg.default.model.as_deref(), Some("opus"));
        assert_eq!(
            cfg.default.permission.get("mode").map(String::as_str),
            Some("bypass")
        );
        let automation = cfg.permission_automation.as_ref().expect("automation");
        assert!(automation.enabled);
        assert!(automation.allowed_tools.iter().any(|item| item == "Read"));
        assert!(
            automation
                .denied_tools
                .iter()
                .any(|item| item == "Bash(rm -rf *)")
        );
        assert!(
            cfg.claude
                .permissions
                .ask
                .iter()
                .any(|item| item == "Write(/etc/*)")
        );
        assert!(
            cfg.claude
                .permissions
                .additional_directories
                .iter()
                .any(|item| item == "/extra")
        );
    }

    #[test]
    fn parse_newer_claude_settings_keys_normal() {
        let settings: ClaudeCompatibilityConfig = serde_json::from_str(
            r#"
            {
              "theme": "dark",
              "outputStyle": "concise",
              "fallbackModel": "claude-sonnet-4-6",
              "defaultShell": "powershell",
              "alwaysThinkingEnabled": true,
              "thinkingBudget": 4096,
              "thinkingDisplay": "summarized",
              "worktree": {
                "baseRef": "origin/main",
                "sparsePaths": ["crates"],
                "symlinkDirectories": ["node_modules"]
              },
              "sandbox": {
                "enabled": true,
                "failIfUnavailable": true,
                "allowUnsandboxedCommands": ["git status"],
                "network": {
                  "allowedDomains": ["example.com"],
                  "allowManagedDomainsOnly": true
                },
                "filesystem": {
                  "allowWrite": ["./target"],
                  "denyWrite": ["~/.ssh"],
                  "allowRead": ["./README.md"],
                  "denyRead": ["~/.aws"]
                },
                "ripgrep": {
                  "command": "rg",
                  "args": ["--pcre2"]
                }
              },
              "skillOverrides": {
                "verify": "off"
              },
              "claudeInChromeDefaultEnabled": true,
              "chromeExtension": {
                "pairedDeviceId": "device-123"
              },
              "disableWorkflows": true,
              "disableRemoteControl": true,
              "disableAllHooks": true
            }
            "#,
        )
        .unwrap();

        assert_eq!(settings.theme.as_deref(), Some("dark"));
        assert_eq!(settings.output_style.as_deref(), Some("concise"));
        assert_eq!(
            settings.fallback_model.as_deref(),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(settings.default_shell.as_deref(), Some("powershell"));
        assert_eq!(settings.thinking_budget, Some(4096));
        assert_eq!(
            settings
                .worktree
                .as_ref()
                .and_then(|w| w.base_ref.as_deref()),
            Some("origin/main")
        );
        let sandbox = settings.sandbox.as_ref().expect("sandbox");
        assert_eq!(sandbox.enabled, Some(true));
        assert_eq!(sandbox.network.allowed_domains, vec!["example.com"]);
        assert_eq!(sandbox.filesystem.deny_write, vec!["~/.ssh"]);
        assert_eq!(sandbox.ripgrep.args, vec!["--pcre2"]);
        assert_eq!(settings.skill_overrides["verify"], "off");
        assert_eq!(settings.claude_in_chrome_default_enabled, Some(true));
        assert_eq!(
            settings
                .chrome_extension
                .as_ref()
                .and_then(|chrome| chrome.paired_device_id.as_deref()),
            Some("device-123")
        );
    }

    #[test]
    fn apply_newer_claude_settings_maps_runtime_config_normal() {
        let mut cfg = Config::default();
        apply_settings(
            &mut cfg,
            ClaudeCompatibilityConfig {
                theme: Some("light".to_owned()),
                output_style: Some("minimal".to_owned()),
                fallback_model: Some("claude-sonnet-4-6".to_owned()),
                default_shell: Some("powershell".to_owned()),
                always_thinking_enabled: Some(true),
                thinking_budget: Some(2048),
                thinking_display: Some("hidden".to_owned()),
                worktree: Some(WorktreeConfig {
                    base_ref: Some("origin/main".to_owned()),
                    sparse_paths: vec!["src".to_owned()],
                    ..Default::default()
                }),
                sandbox: Some(SandboxConfig {
                    enabled: Some(true),
                    fail_if_unavailable: Some(true),
                    ..Default::default()
                }),
                claude_in_chrome_default_enabled: Some(true),
                disable_remote_control: Some(true),
                disable_workflows: Some(true),
                ..Default::default()
            },
        );

        assert_eq!(cfg.theme.as_deref(), Some("light"));
        assert_eq!(cfg.output_style.as_deref(), Some("minimal"));
        assert_eq!(cfg.default_shell.as_deref(), Some("powershell"));
        assert_eq!(cfg.default.thinking_budget, Some(2048));
        assert_eq!(
            cfg.refusal_fallback_model.as_deref(),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(
            cfg.default
                .provider_options
                .get("alwaysThinkingEnabled")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            cfg.default
                .provider_options
                .get("thinkingDisplay")
                .and_then(|v| v.as_str()),
            Some("hidden")
        );
        assert_eq!(
            cfg.worktree.as_ref().and_then(|w| w.base_ref.as_deref()),
            Some("origin/main")
        );
        assert_eq!(cfg.sandbox.as_ref().and_then(|s| s.enabled), Some(true));
        assert!(cfg.remote_control.as_ref().is_some_and(|rc| rc.disabled));
        assert!(cfg.disabled_tools.iter().any(|tool| tool == "Workflow"));
        let chrome = cfg
            .mcp
            .get(CLAUDE_IN_CHROME_MCP_SERVER)
            .expect("chrome MCP server");
        assert_eq!(chrome.server_type.as_deref(), Some("stdio"));
        assert_eq!(chrome.command.as_deref(), Some("claude"));
        assert_eq!(chrome.args, vec![CLAUDE_IN_CHROME_MCP_ARG]);
    }

    #[test]
    fn chrome_auto_enable_respects_denied_mcp_servers_normal() {
        let mut cfg = Config::default();
        apply_settings(
            &mut cfg,
            ClaudeCompatibilityConfig {
                claude_in_chrome_default_enabled: Some(true),
                denied_mcp_servers: vec![CLAUDE_IN_CHROME_MCP_SERVER.to_owned()],
                ..Default::default()
            },
        );

        assert!(!cfg.mcp.contains_key(CLAUDE_IN_CHROME_MCP_SERVER));
    }

    #[test]
    fn chrome_auto_enable_preserves_explicit_mcp_config_normal() {
        let mut cfg = Config::default();
        cfg.mcp.insert(
            CLAUDE_IN_CHROME_MCP_SERVER.to_owned(),
            McpServerConfig {
                server_type: Some("stdio".to_owned()),
                command: Some("custom-claude".to_owned()),
                args: vec!["custom".to_owned()],
                ..Default::default()
            },
        );
        apply_settings(
            &mut cfg,
            ClaudeCompatibilityConfig {
                claude_in_chrome_default_enabled: Some(true),
                ..Default::default()
            },
        );

        let chrome = cfg.mcp.get(CLAUDE_IN_CHROME_MCP_SERVER).unwrap();
        assert_eq!(chrome.command.as_deref(), Some("custom-claude"));
        assert_eq!(chrome.args, vec!["custom"]);
    }

    #[test]
    fn settings_paths_includes_all_three_tiers_normal() {
        let root = std::path::Path::new("/tmp/test-project");
        let paths = settings_paths(root);
        // Must end with project + local; home path may or may not be present.
        let project_path = root.join(".claude").join("settings.json");
        let local_path = root.join(".claude").join("settings.local.json");
        assert!(
            paths.contains(&project_path),
            "missing project settings.json"
        );
        assert!(paths.contains(&local_path), "missing settings.local.json");
        // local must come after project (last-wins merge)
        let proj_idx = paths.iter().position(|p| p == &project_path).unwrap();
        let local_idx = paths.iter().position(|p| p == &local_path).unwrap();
        assert!(
            local_idx > proj_idx,
            "local must override project (wrong order)"
        );
    }

    #[test]
    fn load_merged_local_overrides_project_normal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let claude_dir = root.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        // Project settings: model = "project-model"
        std::fs::write(
            claude_dir.join("settings.json"),
            r#"{"model": "project-model"}"#,
        )
        .unwrap();
        // Local override: model = "local-model"
        std::fs::write(
            claude_dir.join("settings.local.json"),
            r#"{"model": "local-model"}"#,
        )
        .unwrap();
        let merged = load_merged(root);
        assert_eq!(
            merged.model.as_deref(),
            Some("local-model"),
            "settings.local.json must override settings.json"
        );
    }

    #[test]
    fn nested_claude_hooks_parse_to_shell_hooks_normal() {
        let settings: ClaudeCompatibilityConfig = serde_json::from_str(
            r#"
            {
              "hooks": {
                "PostToolUse": [
                  {
                    "matcher": "Write|Edit",
                    "hooks": [
                      { "type": "command", "command": "cargo fmt", "async": true }
                    ]
                  }
                ],
                "UserPromptSubmit": [
                  { "command": "echo prompt" }
                ]
              }
            }
            "#,
        )
        .unwrap();

        let hooks = settings.hooks.expect("hooks");
        assert_eq!(hooks.post_tool_use.len(), 1);
        assert_eq!(
            hooks.post_tool_use[0].matcher.as_deref(),
            Some("Write|Edit")
        );
        assert_eq!(hooks.post_tool_use[0].command, "cargo fmt");
        assert!(hooks.post_tool_use[0].async_mode);
        assert_eq!(hooks.user_prompt_submit[0].command, "echo prompt");
    }

    #[test]
    fn new_hook_events_round_trip_normal() {
        // Verify all CC 2.1.167 new hook events are parsed correctly.
        let json = r#"{
            "hooks": {
                "Setup": [{"command": "echo setup"}],
                "UserPromptExpansion": [{"command": "echo expand"}],
                "MessageDisplay": [{"command": "echo display"}],
                "Elicitation": [{"command": "echo elicit"}],
                "PostToolBatch": [{"command": "echo batch"}],
                "PostCompact": [{"command": "echo compact"}],
                "TaskCreated": [{"command": "echo task-created"}],
                "TaskCompleted": [{"command": "echo task-done"}]
            }
        }"#;
        let settings: ClaudeCompatibilityConfig = serde_json::from_str(json).unwrap();
        let hooks = settings.hooks.expect("hooks");
        assert_eq!(hooks.setup[0].command, "echo setup");
        assert_eq!(hooks.user_prompt_expansion[0].command, "echo expand");
        assert_eq!(hooks.message_display[0].command, "echo display");
        assert_eq!(hooks.elicitation[0].command, "echo elicit");
        assert_eq!(hooks.post_tool_batch[0].command, "echo batch");
        assert_eq!(hooks.post_compact[0].command, "echo compact");
        assert_eq!(hooks.task_created[0].command, "echo task-created");
        assert_eq!(hooks.task_completed[0].command, "echo task-done");
    }

    #[test]
    fn new_settings_fields_round_trip_normal() {
        let json = r#"{
            "autoScrollEnabled": true,
            "showMessageTimestamps": false,
            "autoDreamEnabled": true,
            "autoMemoryEnabled": false,
            "autoCompactEnabled": false,
            "autoCompactWindow": 8000,
            "todoFeatureEnabled": false,
            "plansDirectory": ".plans",
            "effortLevel": "high",
            "fastMode": true,
            "preferredNotifChannel": "slack",
            "teammateMode": "auto"
        }"#;
        let s: ClaudeCompatibilityConfig = serde_json::from_str(json).unwrap();
        assert_eq!(s.auto_scroll_enabled, Some(true));
        assert_eq!(s.show_message_timestamps, Some(false));
        assert_eq!(s.auto_dream_enabled, Some(true));
        assert_eq!(s.auto_memory_enabled, Some(false));
        assert_eq!(s.auto_compact_enabled, Some(false));
        assert_eq!(s.auto_compact_window, Some(8000));
        assert_eq!(s.todo_feature_enabled, Some(false));
        assert_eq!(s.plans_directory.as_deref(), Some(".plans"));
        assert_eq!(s.effort_level.as_deref(), Some("high"));
        assert_eq!(s.fast_mode, Some(true));
        assert_eq!(s.preferred_notif_channel.as_deref(), Some("slack"));
        assert_eq!(s.teammate_mode.as_deref(), Some("auto"));
    }
}
