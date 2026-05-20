//! Model Context Protocol (MCP) support for jfc.
//!
//! MCP is Anthropic's standard for letting Claude call into external
//! tool servers. Each server is a separate process speaking JSON-RPC
//! 2.0 over stdio, with the same `Content-Length: N\r\n\r\n` framing
//! as LSP.

pub mod protocol;
pub mod registry;
pub mod tool_dispatch;
pub mod transport;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use protocol::ToolCallOutcome;
pub use registry::{
    DispatchError, McpRegistry, McpServer, register_servers_from_config, restart_server,
};
pub use tool_dispatch::{
    DEFAULT_DISPATCH_TIMEOUT, dispatch_mcp_tool, dispatch_mcp_tool_with_timeout, is_mcp_tool_name,
};
pub use transport::{RequestError, SpawnConfig, Transport};

/// MCP server definition from user config.
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

/// Convenience: list all currently active (Connected + transport alive) servers.
pub async fn list_active_servers(registry: &McpRegistry) -> Vec<std::sync::Arc<McpServer>> {
    registry.list_active().await
}

/// Convenience: top-level dispatcher.
pub async fn dispatch_tool(
    registry: &McpRegistry,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<ToolCallOutcome, DispatchError> {
    tool_dispatch::dispatch_mcp_tool(registry, tool_name, arguments).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_active_servers_empty_normal() {
        let reg = McpRegistry::new();
        let active = list_active_servers(&reg).await;
        assert!(active.is_empty());
    }

    #[tokio::test]
    async fn dispatch_tool_alias_matches_inner_normal() {
        let reg = McpRegistry::new();
        let res = dispatch_tool(&reg, "Bash", serde_json::json!({})).await;
        assert!(matches!(res, Err(DispatchError::NotMcpName)));
    }
}
