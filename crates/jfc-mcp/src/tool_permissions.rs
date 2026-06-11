//! Per-tool permission gating for MCP servers.
//!
//! Mirrors Perplexity's connector-service tool-permission model found in the
//! 2026-06-11 mindemon dump
//! (`/rest/connector-service/sources/{id}/tool-permissions/{tool_name}`):
//! - "Enable or disable individual tools for your MCP services"
//! - "{enabled} of {total} tools available"
//! - Admin sets a default + per-tool allow/block.
//! - "When switched on, members can re-enable tools blocked by an admin.
//!   Members can always block admin-allowed tools." — i.e. a member override
//!   can loosen an admin *soft* block but never loosen a *hard* block, and can
//!   always tighten (block) anything.
//!
//! The store is keyed by `(server, tool)` and resolves an effective decision by
//! layering: server default → admin per-tool → member per-tool, under the
//! precedence rules above. It's deliberately self-contained and serde-friendly
//! so it can be persisted alongside MCP server config and consulted from
//! `registry::dispatch_tool`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// An admin-level setting for a tool (or the server default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdminSetting {
    /// Tool is allowed by the admin.
    #[default]
    Allow,
    /// Soft block: disabled by default, but a member may re-enable it
    /// ("members can re-enable tools blocked by an admin").
    Block,
    /// Hard block: disabled and members cannot re-enable it.
    HardBlock,
}

/// A member-level override for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberOverride {
    /// Member re-enables a tool (only effective against an admin *soft* block /
    /// or a default-block).
    Enable,
    /// Member blocks a tool (always effective — "members can always block
    /// admin-allowed tools").
    Block,
}

/// The resolved, effective decision for a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDecision {
    Allowed,
    /// Blocked, with a human-readable reason source.
    Blocked(BlockSource),
}

impl ToolDecision {
    pub fn is_allowed(self) -> bool {
        matches!(self, ToolDecision::Allowed)
    }

    pub fn reason(self) -> Option<&'static str> {
        match self {
            ToolDecision::Allowed => None,
            ToolDecision::Blocked(src) => Some(src.reason()),
        }
    }
}

/// Why a tool was blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockSource {
    /// The server default disables tools and nothing re-enabled this one.
    ServerDefault,
    /// An admin soft-blocked it and no member re-enable applies.
    AdminBlock,
    /// An admin hard-blocked it (members cannot re-enable).
    AdminHardBlock,
    /// A member explicitly blocked it.
    MemberBlock,
}

impl BlockSource {
    pub fn reason(self) -> &'static str {
        match self {
            BlockSource::ServerDefault => "disabled by server default",
            BlockSource::AdminBlock => "blocked by admin",
            BlockSource::AdminHardBlock => "hard-blocked by admin (cannot be re-enabled)",
            BlockSource::MemberBlock => "blocked by member setting",
        }
    }
}

/// Per-server tool-permission policy: a default plus per-tool admin settings and
/// member overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerToolPolicy {
    /// Effective state for tools not named explicitly. `Allow` (default) means
    /// tools are available unless individually blocked; `Block` means tools are
    /// off-by-default and must be individually enabled.
    pub default: AdminSetting,
    /// Admin per-tool settings.
    pub admin: HashMap<String, AdminSetting>,
    /// Member per-tool overrides.
    pub member: HashMap<String, MemberOverride>,
}

impl Default for ServerToolPolicy {
    fn default() -> Self {
        Self {
            default: AdminSetting::Allow,
            admin: HashMap::new(),
            member: HashMap::new(),
        }
    }
}

impl ServerToolPolicy {
    /// A policy where every tool is disabled unless individually enabled.
    pub fn deny_by_default() -> Self {
        Self {
            default: AdminSetting::Block,
            ..Self::default()
        }
    }

    pub fn set_admin(&mut self, tool: impl Into<String>, setting: AdminSetting) {
        self.admin.insert(tool.into(), setting);
    }

    pub fn set_member(&mut self, tool: impl Into<String>, ov: MemberOverride) {
        self.member.insert(tool.into(), ov);
    }

    /// Resolve the effective decision for a tool, layering server default →
    /// admin → member with the precedence rules:
    /// 1. A member *block* always wins (members can always block).
    /// 2. An admin *hard block* cannot be loosened by a member.
    /// 3. A member *enable* loosens an admin soft-block or a default-block.
    /// 4. Otherwise the admin per-tool setting, then the server default.
    pub fn decide(&self, tool: &str) -> ToolDecision {
        // (1) Member block always wins.
        if matches!(self.member.get(tool), Some(MemberOverride::Block)) {
            return ToolDecision::Blocked(BlockSource::MemberBlock);
        }

        let admin = self.admin.get(tool).copied().unwrap_or(self.default);
        let member_enable = matches!(self.member.get(tool), Some(MemberOverride::Enable));

        match admin {
            // (2) Hard block: members cannot re-enable.
            AdminSetting::HardBlock => ToolDecision::Blocked(BlockSource::AdminHardBlock),
            // (3) Soft block: a member enable loosens it.
            AdminSetting::Block => {
                if member_enable {
                    ToolDecision::Allowed
                } else if self.admin.contains_key(tool) {
                    ToolDecision::Blocked(BlockSource::AdminBlock)
                } else {
                    // Came from the server default (default == Block).
                    ToolDecision::Blocked(BlockSource::ServerDefault)
                }
            }
            AdminSetting::Allow => ToolDecision::Allowed,
        }
    }

    /// Whether a tool is currently allowed.
    pub fn is_allowed(&self, tool: &str) -> bool {
        self.decide(tool).is_allowed()
    }

    /// "{enabled} of {total}" — count allowed tools out of the supplied set.
    pub fn enabled_count(&self, all_tools: &[String]) -> (usize, usize) {
        let enabled = all_tools.iter().filter(|t| self.is_allowed(t)).count();
        (enabled, all_tools.len())
    }
}

/// A store of per-server tool policies, keyed by MCP server name. Servers
/// without an explicit policy are fully allowed (default-open, matching JFC's
/// existing behaviour of dispatching any advertised tool).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPermissionStore {
    servers: HashMap<String, ServerToolPolicy>,
}

impl ToolPermissionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_policy(&mut self, server: impl Into<String>, policy: ServerToolPolicy) {
        self.servers.insert(server.into(), policy);
    }

    pub fn policy(&self, server: &str) -> Option<&ServerToolPolicy> {
        self.servers.get(server)
    }

    pub fn policy_mut(&mut self, server: &str) -> &mut ServerToolPolicy {
        self.servers.entry(server.to_owned()).or_default()
    }

    /// Resolve the decision for `(server, tool)`. Servers with no policy are
    /// allowed (default-open).
    pub fn decide(&self, server: &str, tool: &str) -> ToolDecision {
        match self.servers.get(server) {
            Some(policy) => policy.decide(tool),
            None => ToolDecision::Allowed,
        }
    }

    pub fn is_allowed(&self, server: &str, tool: &str) -> bool {
        self.decide(server, tool).is_allowed()
    }

    /// Decide using an advertised `mcp__server__tool` name. Returns `Allowed`
    /// for names that aren't MCP tools (the native path handles those).
    pub fn decide_advertised(&self, advertised: &str) -> ToolDecision {
        match super::protocol::split_advertised(advertised) {
            Some((server, tool)) => self.decide(server, tool),
            None => ToolDecision::Allowed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Default-open ─────────────────────────────────────────────────────────

    #[test]
    fn unknown_server_is_allowed_normal() {
        let store = ToolPermissionStore::new();
        assert!(store.is_allowed("nope", "any_tool"));
        assert_eq!(store.decide("nope", "any_tool"), ToolDecision::Allowed);
    }

    #[test]
    fn default_allow_policy_allows_unnamed_tools_normal() {
        let policy = ServerToolPolicy::default();
        assert!(policy.is_allowed("read_file"));
    }

    // ── Admin settings ─────────────────────────────────────────────────────────

    #[test]
    fn admin_block_disables_a_tool_normal() {
        let mut policy = ServerToolPolicy::default();
        policy.set_admin("dangerous", AdminSetting::Block);
        let decision = policy.decide("dangerous");
        assert_eq!(decision, ToolDecision::Blocked(BlockSource::AdminBlock));
        assert!(policy.is_allowed("safe"));
    }

    #[test]
    fn deny_by_default_blocks_unnamed_tools_normal() {
        let policy = ServerToolPolicy::deny_by_default();
        assert_eq!(
            policy.decide("anything"),
            ToolDecision::Blocked(BlockSource::ServerDefault)
        );
    }

    // ── Member overrides + precedence ──────────────────────────────────────────

    #[test]
    fn member_can_reenable_admin_soft_block_normal() {
        // "members can re-enable tools blocked by an admin"
        let mut policy = ServerToolPolicy::default();
        policy.set_admin("tool_x", AdminSetting::Block);
        assert!(!policy.is_allowed("tool_x"));
        policy.set_member("tool_x", MemberOverride::Enable);
        assert!(policy.is_allowed("tool_x"));
    }

    #[test]
    fn member_cannot_reenable_hard_block_robust() {
        let mut policy = ServerToolPolicy::default();
        policy.set_admin("tool_x", AdminSetting::HardBlock);
        policy.set_member("tool_x", MemberOverride::Enable);
        assert_eq!(
            policy.decide("tool_x"),
            ToolDecision::Blocked(BlockSource::AdminHardBlock)
        );
    }

    #[test]
    fn member_can_always_block_admin_allowed_normal() {
        // "Members can always block admin-allowed tools."
        let mut policy = ServerToolPolicy::default();
        policy.set_admin("tool_x", AdminSetting::Allow);
        policy.set_member("tool_x", MemberOverride::Block);
        assert_eq!(
            policy.decide("tool_x"),
            ToolDecision::Blocked(BlockSource::MemberBlock)
        );
    }

    #[test]
    fn member_block_beats_everything_robust() {
        // Even a hard-allowed default tool is blocked by a member block.
        let mut policy = ServerToolPolicy::deny_by_default();
        policy.set_member("tool_x", MemberOverride::Enable);
        // Member enabled it against the default-block...
        assert!(policy.is_allowed("tool_x"));
        // ...then blocks it — block wins.
        policy.set_member("tool_x", MemberOverride::Block);
        assert!(!policy.is_allowed("tool_x"));
    }

    #[test]
    fn member_enable_loosens_default_block_normal() {
        let mut policy = ServerToolPolicy::deny_by_default();
        policy.set_member("tool_x", MemberOverride::Enable);
        assert!(policy.is_allowed("tool_x"));
        // A different tool stays blocked by default.
        assert!(!policy.is_allowed("other"));
    }

    // ── Counts + store ─────────────────────────────────────────────────────────

    #[test]
    fn enabled_count_reports_fraction_normal() {
        let mut policy = ServerToolPolicy::default();
        policy.set_admin("b", AdminSetting::Block);
        let tools = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(policy.enabled_count(&tools), (2, 3));
    }

    #[test]
    fn store_decide_advertised_routes_by_name_normal() {
        let mut store = ToolPermissionStore::new();
        let mut policy = ServerToolPolicy::default();
        policy.set_admin("write_file", AdminSetting::HardBlock);
        store.set_policy("filesystem", policy);

        assert!(!store.is_allowed("filesystem", "write_file"));
        assert!(store.is_allowed("filesystem", "read_file"));
        // Advertised-name routing.
        assert!(
            !store
                .decide_advertised("mcp__filesystem__write_file")
                .is_allowed()
        );
        assert!(
            store
                .decide_advertised("mcp__filesystem__read_file")
                .is_allowed()
        );
        // Non-MCP names pass through.
        assert!(store.decide_advertised("Bash").is_allowed());
    }

    #[test]
    fn store_roundtrips_serde_robust() {
        let mut store = ToolPermissionStore::new();
        let mut policy = ServerToolPolicy::deny_by_default();
        policy.set_admin("x", AdminSetting::HardBlock);
        policy.set_member("y", MemberOverride::Enable);
        store.set_policy("srv", policy);

        let json = serde_json::to_string(&store).unwrap();
        let back: ToolPermissionStore = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.decide("srv", "x"),
            ToolDecision::Blocked(BlockSource::AdminHardBlock)
        );
        assert!(back.is_allowed("srv", "y"));
        assert!(!back.is_allowed("srv", "z")); // default block
    }
}
