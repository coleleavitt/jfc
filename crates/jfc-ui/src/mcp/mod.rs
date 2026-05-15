//! Model Context Protocol (MCP) support for jfc.
//!
//! MCP is Anthropic's standard for letting Claude call into external
//! tool servers. Each server is a separate process speaking JSON-RPC
//! 2.0 over stdio, with the same `Content-Length: N\r\n\r\n` framing
//! as LSP. Servers advertise tools via `tools/list`; the client invokes
//! them via `tools/call`. Tool names are namespaced
//! `mcp__<server>__<tool>` so they don't collide with jfc's native
//! tool catalogue.
//!
//! ## Module layout
//!
//! - [`protocol`] — JSON-RPC 2.0 + MCP message shapes (initialize,
//!   tools/list, tools/call, notifications). Pure parse/build, no I/O.
//! - [`transport`] — process-level transport: spawn the server,
//!   stdio framing, request/response routing.
//! - [`registry`] — keeps [`McpServer`] entries in a `RwLock<HashMap>`,
//!   spawns from config, supports restart.
//! - [`tool_dispatch`] — entry point the streaming layer calls when a
//!   tool name matches `mcp__server__tool`.
//!
//! ## Wiring (called from `main.rs` startup)
//!
//! ```ignore
//! let registry = mcp::McpRegistry::new();
//! mcp::register_servers_from_config(&registry, &config.mcp).await;
//! ```
//!
//! and at tool advertise time (in `stream.rs`):
//!
//! ```ignore
//! let mut tools = tools::all_tool_defs();
//! tools.extend(registry.all_advertised_tool_defs().await);
//! ```
//!
//! and at tool dispatch time (in `tools.rs::execute_tool` or
//! `stream.rs`'s tool_use handler):
//!
//! ```ignore
//! if mcp::tool_dispatch::is_mcp_tool_name(name) {
//!     let outcome = mcp::tool_dispatch::dispatch_mcp_tool(&registry, name, args).await?;
//!     // outcome.text → tool result content; outcome.is_error → flag the failure
//! }
//! ```

#![allow(dead_code)]

pub mod protocol;
pub mod registry;
pub mod tool_dispatch;
pub mod transport;

pub use protocol::{ToolCallOutcome, split_advertised};
pub use registry::{
    DispatchError, McpRegistry, McpServer, register_servers_from_config, restart_server,
};
#[allow(unused_imports)]
pub use tool_dispatch::{
    DEFAULT_DISPATCH_TIMEOUT, dispatch_mcp_tool, dispatch_mcp_tool_with_timeout, is_mcp_tool_name,
};
#[allow(unused_imports)]
pub use transport::{RequestError, SpawnConfig, Transport};

/// Convenience: list all currently active (Connected + transport alive)
/// servers. Equivalent to `registry.list_active().await`. Exists at the
/// module root because `/mcp list` reaches for it directly.
pub async fn list_active_servers(registry: &McpRegistry) -> Vec<std::sync::Arc<McpServer>> {
    registry.list_active().await
}

/// Convenience: top-level dispatcher matching the deliverable spec.
/// Same body as [`tool_dispatch::dispatch_mcp_tool`]; re-exported here
/// so callers can `mcp::dispatch_tool(...)` per the brief.
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
