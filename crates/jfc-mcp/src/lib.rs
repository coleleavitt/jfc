//! Model Context Protocol (MCP) support for jfc.
//!
//! MCP is Anthropic's standard for letting Claude call into external
//! tool servers. Wire-level protocol (JSON-RPC 2.0 framing, the
//! `initialize` handshake, `tools/list` pagination) is handled by the
//! [`rmcp`] SDK; this crate owns jfc's product behavior on top of it:
//! the [`McpRegistry`] of live connections, the `mcp__<server>__<tool>`
//! advertised-name scheme, `/mcp list|restart|logs`, and the
//! stdio-vs-streamable-HTTP transport selection driven by config.

pub mod protocol;
pub mod registry;
pub mod tool_dispatch;
pub mod transport;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use protocol::ToolCallOutcome;
pub use registry::{
    DispatchError, McpRegistry, McpResource, McpServer, McpServerStatus,
    register_servers_from_config, restart_server,
};
pub use tool_dispatch::{
    DEFAULT_DISPATCH_TIMEOUT, dispatch_mcp_tool, dispatch_mcp_tool_with_timeout, is_mcp_tool_name,
};
pub use transport::{RequestError, SpawnConfig, Transport, TransportKind};

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
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl McpServerConfig {
    fn command_present(&self) -> bool {
        self.command
            .as_deref()
            .is_some_and(|c| !c.trim().is_empty())
    }

    fn url_present(&self) -> bool {
        self.url.as_deref().is_some_and(|u| !u.trim().is_empty())
    }

    /// Resolve which transport to spawn. The `type` field is
    /// authoritative when present: `stdio` requires a `command`; `http`
    /// (aliases: `streamable-http`, `streamable_http`, `streamablehttp`)
    /// and the legacy `sse` both require a `url` and use rmcp's
    /// streamable-HTTP client. rmcp 1.7 has no standalone SSE client, so
    /// `sse` is accepted as an alias and logged. An unrecognized `type`,
    /// or a recognized one missing its required field, is a hard error
    /// (the server is marked `Failed`). When `type` is omitted we infer:
    /// a `url` means HTTP, otherwise a `command` means stdio.
    pub fn resolve_transport(&self) -> Result<TransportKind, String> {
        let declared = self
            .server_type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);

        match declared.as_deref() {
            Some("stdio") => {
                if self.command_present() {
                    Ok(TransportKind::Stdio)
                } else {
                    Err("type = \"stdio\" requires a `command`".to_owned())
                }
            }
            Some("http" | "streamable-http" | "streamable_http" | "streamablehttp") => {
                if self.url_present() {
                    Ok(TransportKind::Http)
                } else {
                    Err("type = \"http\" requires a `url`".to_owned())
                }
            }
            Some("sse") => {
                if self.url_present() {
                    tracing::warn!(
                        target: "jfc::mcp",
                        "type = \"sse\" — rmcp has no standalone SSE client; \
                         using the streamable-HTTP transport against the url"
                    );
                    Ok(TransportKind::Http)
                } else {
                    Err("type = \"sse\" requires a `url`".to_owned())
                }
            }
            Some(other) => Err(format!(
                "unknown type = \"{other}\" (expected \"stdio\", \"http\", or \"sse\")"
            )),
            None => {
                if self.url_present() {
                    Ok(TransportKind::Http)
                } else if self.command_present() {
                    Ok(TransportKind::Stdio)
                } else {
                    Err("no `command` or `url` configured".to_owned())
                }
            }
        }
    }
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

    fn cfg(server_type: Option<&str>, command: Option<&str>, url: Option<&str>) -> McpServerConfig {
        McpServerConfig {
            server_type: server_type.map(str::to_owned),
            command: command.map(str::to_owned),
            args: Vec::new(),
            env: HashMap::new(),
            headers: HashMap::new(),
            url: url.map(str::to_owned),
        }
    }

    #[test]
    fn resolve_explicit_stdio_normal() {
        let c = cfg(Some("stdio"), Some("npx"), None);
        assert_eq!(c.resolve_transport(), Ok(TransportKind::Stdio));
    }

    #[test]
    fn resolve_explicit_http_aliases_normal() {
        for t in [
            "http",
            "streamable-http",
            "streamable_http",
            "STREAMABLEHTTP",
        ] {
            let c = cfg(Some(t), None, Some("https://example.com/mcp"));
            assert_eq!(c.resolve_transport(), Ok(TransportKind::Http), "type={t}");
        }
    }

    #[test]
    fn resolve_sse_maps_to_http_normal() {
        let c = cfg(Some("sse"), None, Some("https://example.com/sse"));
        assert_eq!(c.resolve_transport(), Ok(TransportKind::Http));
    }

    #[test]
    fn resolve_type_is_authoritative_over_inference_robust() {
        // Declared stdio but only a url present → error, not silent HTTP.
        let c = cfg(Some("stdio"), None, Some("https://example.com/mcp"));
        assert!(c.resolve_transport().is_err());
        // Declared http but only a command present → error, not silent stdio.
        let c = cfg(Some("http"), Some("npx"), None);
        assert!(c.resolve_transport().is_err());
    }

    #[test]
    fn resolve_unknown_type_is_error_robust() {
        let c = cfg(Some("grpc"), Some("npx"), Some("https://x"));
        assert!(c.resolve_transport().is_err());
    }

    #[test]
    fn resolve_infers_when_type_omitted_normal() {
        assert_eq!(
            cfg(None, Some("npx"), None).resolve_transport(),
            Ok(TransportKind::Stdio)
        );
        assert_eq!(
            cfg(None, None, Some("https://x/mcp")).resolve_transport(),
            Ok(TransportKind::Http)
        );
        // url wins when both are present and type is omitted.
        assert_eq!(
            cfg(None, Some("npx"), Some("https://x/mcp")).resolve_transport(),
            Ok(TransportKind::Http)
        );
    }

    #[test]
    fn resolve_empty_config_is_error_robust() {
        assert!(cfg(None, None, None).resolve_transport().is_err());
        // Whitespace-only fields don't count as present.
        assert!(
            cfg(Some("stdio"), Some("   "), None)
                .resolve_transport()
                .is_err()
        );
    }
}
