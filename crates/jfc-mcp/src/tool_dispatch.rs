//! Tool-call routing for advertised MCP tools.
//!
//! When the model invokes a tool whose name matches `mcp__<server>__<tool>`,
//! the streaming layer drops out of native dispatch and calls
//! [`dispatch_mcp_tool`] from this module. Native tools (`Bash`,
//! `Read`, etc.) still go through `tools.rs::execute_tool`.
//!
//! ## Result shape
//!
//! Native tools return a plain `String` content block. MCP `tools/call`
//! returns `{content: [...], isError: bool}`. We collapse the textual
//! content into a string (via [`super::protocol::parse_tools_call_result`])
//! and propagate `isError` separately so the streaming layer can decide
//! whether to surface it as a tool failure.

use serde_json::Value;

use super::protocol::ToolCallOutcome;
use super::registry::{DispatchError, McpRegistry};

/// Default per-tool-call timeout. MCP servers can take a while
/// (filesystem traversal, GitHub API rate limits) so we lean generous.
pub const DEFAULT_DISPATCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Returns true if `tool_name` is the advertised `mcp__server__tool`
/// shape. Streaming-layer can call this to decide whether to route via
/// MCP or native.
#[allow(dead_code)]
pub fn is_mcp_tool_name(tool_name: &str) -> bool {
    super::protocol::split_advertised(tool_name).is_some()
}

/// Route an advertised MCP tool call to its server. The caller has
/// already confirmed via [`is_mcp_tool_name`] that this is an MCP tool.
///
/// `arguments` is the raw JSON the model produced. We do no schema
/// validation here — the server validates against its own
/// `inputSchema`, and a 400-shaped JSON-RPC error comes back if it
/// doesn't match.
pub async fn dispatch_mcp_tool(
    registry: &McpRegistry,
    tool_name: &str,
    arguments: Value,
) -> Result<ToolCallOutcome, DispatchError> {
    dispatch_mcp_tool_with_timeout(registry, tool_name, arguments, DEFAULT_DISPATCH_TIMEOUT).await
}

/// Same as [`dispatch_mcp_tool`] with a caller-controlled timeout.
pub async fn dispatch_mcp_tool_with_timeout(
    registry: &McpRegistry,
    tool_name: &str,
    arguments: Value,
    timeout: std::time::Duration,
) -> Result<ToolCallOutcome, DispatchError> {
    tracing::debug!(
        target: "jfc::mcp",
        tool_name = %tool_name,
        timeout_secs = timeout.as_secs(),
        "dispatch_mcp_tool"
    );
    // Consult the process-global per-tool permission store (no-op when none is
    // installed). A blocked tool is rejected before the server is contacted.
    if let Some((server, tool)) = super::protocol::split_advertised(tool_name) {
        if let super::tool_permissions::ToolDecision::Blocked(src) =
            super::tool_permissions::active_decision(server, tool)
        {
            return Err(super::registry::DispatchError::ToolBlocked {
                server: server.to_owned(),
                tool: tool.to_owned(),
                reason: src.reason(),
            });
        }
    }
    registry.dispatch_tool(tool_name, arguments, timeout).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn is_mcp_tool_name_normal() {
        assert!(is_mcp_tool_name("mcp__filesystem__read_file"));
        assert!(is_mcp_tool_name("mcp__git__status"));
    }

    #[test]
    fn is_mcp_tool_name_rejects_native_robust() {
        assert!(!is_mcp_tool_name("Bash"));
        assert!(!is_mcp_tool_name("Read"));
        assert!(!is_mcp_tool_name("mcp__"));
        assert!(!is_mcp_tool_name(""));
    }

    #[tokio::test]
    async fn dispatch_unknown_server_returns_error_robust() {
        let reg = McpRegistry::new();
        let res = dispatch_mcp_tool(&reg, "mcp__nope__x", json!({})).await;
        assert!(matches!(res, Err(DispatchError::UnknownServer(_))));
    }

    #[tokio::test]
    async fn dispatch_non_mcp_name_routes_to_not_mcp_robust() {
        let reg = McpRegistry::new();
        let res = dispatch_mcp_tool(&reg, "Bash", json!({})).await;
        assert!(matches!(res, Err(DispatchError::NotMcpName)));
    }

    #[tokio::test]
    async fn dispatch_respects_global_permission_block_normal() {
        use crate::tool_permissions::{
            AdminSetting, ServerToolPolicy, ToolPermissionStore, set_active_permissions,
        };

        // Install a global store that hard-blocks fs/write_file.
        let mut store = ToolPermissionStore::new();
        let mut policy = ServerToolPolicy::default();
        policy.set_admin("write_file", AdminSetting::HardBlock);
        store.set_policy("fs", policy);
        set_active_permissions(Some(store));

        let reg = McpRegistry::new();
        // The blocked tool is rejected before the (absent) server is contacted:
        // ToolBlocked, not UnknownServer.
        let blocked = dispatch_mcp_tool(&reg, "mcp__fs__write_file", json!({})).await;
        assert!(
            matches!(blocked, Err(DispatchError::ToolBlocked { .. })),
            "expected ToolBlocked, got {blocked:?}"
        );

        // A non-blocked tool falls through to normal dispatch (UnknownServer
        // here, since no server is registered).
        let allowed = dispatch_mcp_tool(&reg, "mcp__fs__read_file", json!({})).await;
        assert!(
            matches!(allowed, Err(DispatchError::UnknownServer(_))),
            "expected UnknownServer, got {allowed:?}"
        );

        // Clear the global so other tests see open dispatch.
        set_active_permissions(None);
    }
}
