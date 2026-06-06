//! Agent definition types shared across crates.
//!
//! These are pure data types (serde-derivable, no behavior beyond Display)
//! that multiple crates need: jfc-daemon for persistence, jfc for dispatch.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Agent definition loaded from `.claude/agents/*.md` frontmatter or
/// constructed programmatically for built-in agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    pub name: String,
    pub source: PathBuf,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, rename = "allowedTools")]
    pub allowed_tools: Vec<String>,
    #[serde(default, rename = "disallowedTools")]
    pub disallowed_tools: Vec<String>,
    #[serde(default, rename = "permissionMode")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default, rename = "forksParentContext")]
    pub forks_parent_context: Option<serde_json::Value>,
    #[serde(default)]
    pub background: Option<bool>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default, rename = "maxTurns")]
    pub max_turns: Option<u32>,
    #[serde(default, rename = "maxInputTokens")]
    pub max_input_tokens: Option<u64>,
    #[serde(default)]
    pub memory: Option<MemoryScope>,
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub hooks: HashMap<String, Vec<String>>,
    #[serde(default, rename = "keyTrigger")]
    pub key_trigger: Option<String>,
    #[serde(default, rename = "useWhen")]
    pub use_when: Vec<String>,
    #[serde(default, rename = "avoidWhen")]
    pub avoid_when: Vec<String>,
    #[serde(default)]
    pub cost: Option<AgentCost>,
    pub system_prompt: String,
}

/// Cost tier for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentCost {
    Free,
    Cheap,
    Expensive,
}

/// Reasoning effort level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

/// Memory storage scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    User,
    Project,
    Local,
}

/// Permission mode — controls how tool calls are gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    #[default]
    Default,
    AcceptEdits,
    BypassPermissions,
    Plan,
    DontAsk,
    Auto,
}
