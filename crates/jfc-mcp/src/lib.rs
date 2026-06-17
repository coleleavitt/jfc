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
pub mod tool_permissions;
pub mod transport;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use protocol::ToolCallOutcome;
pub use registry::{
    DispatchError, McpAdvertisedToolMetadata, McpRegistry, McpResource, McpServer, McpServerStatus,
    build_server, register_servers_from_config, restart_server,
};
pub use tool_dispatch::{
    DEFAULT_DISPATCH_TIMEOUT, dispatch_mcp_tool, dispatch_mcp_tool_with_timeout, is_mcp_tool_name,
};
pub use tool_permissions::{
    AdminSetting, BlockSource, MemberOverride, ServerToolPolicy, ToolDecision, ToolPermissionStore,
    active_decision, set_active_permissions,
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
    /// Optional path to a `.env` file whose variables are merged into
    /// `env` before the server is spawned. Variables already present in
    /// `env` take precedence (`.env` only fills gaps).
    #[serde(default, rename = "envFile")]
    pub env_file: Option<std::path::PathBuf>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
    /// Optional OAuth/client metadata for HTTP MCP servers. The current rmcp
    /// transport still expects callers to supply Authorization headers, but
    /// preserving these aliases keeps config compatible with MCP protected-
    /// resource metadata and with clients that spell client id differently.
    #[serde(default, alias = "oauthMetadata", alias = "oauth_metadata")]
    pub oauth: McpOAuthMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct McpOAuthMetadata {
    #[serde(alias = "clientId", alias = "client_id")]
    pub client_id: Option<String>,
    #[serde(alias = "clientSecret", alias = "client_secret")]
    pub client_secret: Option<String>,
    #[serde(alias = "clientSecretEnv", alias = "client_secret_env")]
    pub client_secret_env: Option<String>,
    #[serde(alias = "accessToken", alias = "access_token")]
    pub access_token: Option<String>,
    #[serde(
        alias = "accessTokenEnv",
        alias = "access_token_env",
        alias = "tokenEnv",
        alias = "token_env"
    )]
    pub access_token_env: Option<String>,
    #[serde(
        alias = "tokenUrl",
        alias = "token_url",
        alias = "tokenEndpoint",
        alias = "token_endpoint"
    )]
    pub token_url: Option<String>,
    #[serde(alias = "scope", deserialize_with = "deserialize_oauth_scopes")]
    pub scopes: Vec<String>,
    pub resource: Option<String>,
    #[serde(alias = "authorizationServer", alias = "authorization_server")]
    pub authorization_server: Option<String>,
}

fn deserialize_oauth_scopes<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ScopeField {
        One(String),
        Many(Vec<String>),
    }

    let Some(field) = Option::<ScopeField>::deserialize(deserializer)? else {
        return Ok(Vec::new());
    };
    let scopes = match field {
        ScopeField::One(scope) => scope
            .split_whitespace()
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned)
            .collect(),
        ScopeField::Many(scopes) => scopes
            .into_iter()
            .map(|scope| scope.trim().to_owned())
            .filter(|scope| !scope.is_empty())
            .collect(),
    };
    Ok(scopes)
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

    pub fn oauth_resource(&self) -> Option<String> {
        self.oauth
            .resource
            .as_deref()
            .or(self.url.as_deref())
            .map(normalize_oauth_resource)
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

fn normalize_oauth_resource(resource: &str) -> String {
    let trimmed = resource.trim();
    if trimmed.is_empty() || trimmed.ends_with('/') {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/")
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
            env_file: None,
            headers: HashMap::new(),
            url: url.map(str::to_owned),
            oauth: McpOAuthMetadata::default(),
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

    #[test]
    fn oauth_metadata_accepts_aliases_and_normalizes_resource_regression() {
        let cfg: McpServerConfig = serde_json::from_value(serde_json::json!({
            "type": "http",
            "url": "https://mcp.example.test/mcp",
            "oauth": {
                "clientId": "client-a",
                "scope": "tools.read tools.write",
                "authorizationServer": "https://auth.example.test"
            }
        }))
        .expect("config parses");
        assert_eq!(cfg.oauth.client_id.as_deref(), Some("client-a"));
        assert_eq!(cfg.oauth.access_token_env.as_deref(), None);
        assert_eq!(cfg.oauth.scopes, vec!["tools.read", "tools.write"]);
        assert_eq!(
            cfg.oauth.authorization_server.as_deref(),
            Some("https://auth.example.test")
        );
        assert_eq!(
            cfg.oauth_resource().as_deref(),
            Some("https://mcp.example.test/mcp/")
        );

        let snake: McpServerConfig = serde_json::from_value(serde_json::json!({
            "type": "http",
            "url": "https://mcp.example.test/mcp/",
            "oauth_metadata": {
                "client_id": "client-b",
                "tokenUrl": "https://auth.example.test/token",
                "clientSecretEnv": "MCP_CLIENT_SECRET",
                "accessTokenEnv": "MCP_ACCESS_TOKEN",
                "scopes": ["tools.read"],
                "resource": "https://resource.example.test/root"
            }
        }))
        .expect("snake config parses");
        assert_eq!(snake.oauth.client_id.as_deref(), Some("client-b"));
        assert_eq!(
            snake.oauth.token_url.as_deref(),
            Some("https://auth.example.test/token")
        );
        assert_eq!(
            snake.oauth.client_secret_env.as_deref(),
            Some("MCP_CLIENT_SECRET")
        );
        assert_eq!(
            snake.oauth.access_token_env.as_deref(),
            Some("MCP_ACCESS_TOKEN")
        );
        assert_eq!(snake.oauth.scopes, vec!["tools.read"]);
        assert_eq!(
            snake.oauth_resource().as_deref(),
            Some("https://resource.example.test/root/")
        );
    }
}
