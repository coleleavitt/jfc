//! In-process registry of live MCP server connections.
//!
//! Each [`McpServer`] owns a [`Transport`] handle plus a cached tool list
//! and a status flag. The registry holds them in a
//! `RwLock<HashMap<name, Arc<McpServer>>>` so:
//!
//! - **Read-heavy paths** (the streaming send-loop checking which tools
//!   to advertise) take the read lock and clone an `Arc<McpServer>`.
//! - **Write paths** (server spawn, restart, removal) take the write
//!   lock briefly to swap in/out the entry.
//!
//! Restart works by removing the current entry, dropping it (which
//! `kill_on_drop`s the child via the transport), then spawning fresh.
//! The dispatcher always pulls the *current* server from the registry
//! by name, so a tool call mid-restart sees the new transport
//! transparently — modulo a window where `dispatch_tool` returns
//! `Disconnected` if the call lands during the swap.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::McpServerConfig;
use jfc_provider::ToolDef;

use super::protocol::{self, McpTool, ToolCallOutcome};
use super::transport::{RequestError, SpawnConfig, Transport, TransportKind};

/// An MCP resource entry (from `resources/list`).
#[derive(Debug, Clone)]
pub struct McpResource {
    pub name: String,
    pub uri: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// Status of an MCP server entry. Drives the `/mcp list` display and
/// the [`crate::types::McpServerInfo`] sidebar block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpServerStatus {
    /// Handshake completed, tool list cached, ready to dispatch.
    Connected,
    /// Spawn or handshake failed; entry kept so `/mcp list` shows the
    /// configured-but-broken state.
    Failed,
    /// Server explicitly disabled in config (`enabled = false` — not
    /// yet wired but reserved).
    #[allow(dead_code)]
    Disabled,
}

impl McpServerStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Failed => "failed",
            Self::Disabled => "disabled",
        }
    }
}

/// One live or stub MCP server entry.
pub struct McpServer {
    pub name: String,
    pub status: McpServerStatus,
    /// Cached tool list from the most recent `tools/list`. Empty for
    /// `Failed` / `Disabled` entries.
    pub tools: Vec<McpTool>,
    /// Cached resource list from `resources/list`. Empty when unsupported.
    pub resources: Vec<McpResource>,
    /// `None` for `Failed` / `Disabled`. Otherwise the live transport.
    pub transport: Option<Transport>,
    /// Original spawn config — kept so `/mcp restart` can re-spawn
    /// without needing the caller to re-thread the config.
    pub spawn_cfg: SpawnConfig,
}

impl McpServer {
    /// Build the [`ToolDef`] entries we advertise to the model. Each
    /// tool's name is namespaced to `mcp__<server>__<tool>` so dispatch
    /// can route back to the right server.
    pub fn advertised_tool_defs(&self) -> Vec<ToolDef> {
        self.tools
            .iter()
            .map(|t| ToolDef {
                name: protocol::advertise_tool_name(&self.name, &t.name),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
            })
            .collect()
    }
}

impl std::fmt::Debug for McpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServer")
            .field("name", &self.name)
            .field("status", &self.status)
            .field("tool_count", &self.tools.len())
            .field("connected", &self.transport.is_some())
            .finish()
    }
}

/// Registry handle. Cloneable; all clones share the same underlying
/// `RwLock`.
#[derive(Clone, Default)]
pub struct McpRegistry {
    inner: Arc<RwLock<HashMap<String, Arc<McpServer>>>>,
}

/// Process-global "tools/list_changed" signal — incremented every time a
/// server pushes the `notifications/tools/list_changed` notification.
/// The Tick handler checks this and emits a UI toast + invalidates the
/// per-server tool cache so the next stream sees the fresh catalog.
static REFRESH_PENDING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bump the refresh counter. Cheap, non-blocking — called from the
/// MCP transport on inbound notifications.
pub fn request_refresh() {
    REFRESH_PENDING.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Snapshot the refresh counter. Tick handler compares against its
/// last-seen value to detect new notifications.
pub fn refresh_counter() -> u64 {
    REFRESH_PENDING.load(std::sync::atomic::Ordering::SeqCst)
}

impl McpRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add (or replace) a server entry under `name`.
    pub async fn insert(&self, server: McpServer) {
        let name = server.name.clone();
        let mut guard = self.inner.write().await;
        guard.insert(name, Arc::new(server));
    }

    /// Remove an entry by name. Returns the dropped server so callers
    /// can `.shutdown().await` on it before letting it go.
    pub async fn remove(&self, name: &str) -> Option<Arc<McpServer>> {
        let mut guard = self.inner.write().await;
        guard.remove(name)
    }

    /// Look up a server by name. Returns an `Arc` clone so the lock is
    /// released immediately.
    pub async fn get(&self, name: &str) -> Option<Arc<McpServer>> {
        let guard = self.inner.read().await;
        guard.get(name).map(Arc::clone)
    }

    /// All servers, in arbitrary order. Returns `Arc` clones so the
    /// lock is released immediately.
    pub async fn list(&self) -> Vec<Arc<McpServer>> {
        let guard = self.inner.read().await;
        guard.values().map(Arc::clone).collect()
    }

    /// All connected (status = Connected, transport is Some) servers.
    /// Convenience for the dispatcher and `list_active_servers`.
    pub async fn list_active(&self) -> Vec<Arc<McpServer>> {
        let guard = self.inner.read().await;
        guard
            .values()
            .filter(|s| s.status == McpServerStatus::Connected && s.transport.is_some())
            .map(Arc::clone)
            .collect()
    }

    /// Aggregate every connected server's advertised tool defs into a
    /// single Vec the streaming layer can append to `all_tool_defs()`.
    pub async fn all_advertised_tool_defs(&self) -> Vec<ToolDef> {
        let active = self.list_active().await;
        let mut out = Vec::new();
        for s in active {
            out.extend(s.advertised_tool_defs());
        }
        out
    }

    /// Read a resource from a specific server by URI.
    /// Returns the resource content as a string or an error.
    pub async fn read_resource(
        &self,
        server_name: &str,
        uri: &str,
    ) -> Result<String, DispatchError> {
        let server = self
            .get(server_name)
            .await
            .ok_or_else(|| DispatchError::UnknownServer(server_name.to_owned()))?;
        let transport = server
            .transport
            .as_ref()
            .ok_or_else(|| DispatchError::ServerNotConnected(server_name.to_owned()))?;
        let result = transport
            .read_resource(uri)
            .await
            .map_err(DispatchError::Request)?;
        // Extract text from the first content item
        use rmcp::model::ResourceContents;
        let text = result
            .contents
            .into_iter()
            .next()
            .map(|c| match c {
                ResourceContents::TextResourceContents { text, .. } => text,
                ResourceContents::BlobResourceContents { blob, .. } => blob,
            })
            .unwrap_or_default();
        Ok(text)
    }

    /// Dispatch a tool call to the server identified by the
    /// `mcp__<server>__<tool>` name. Returns
    /// `Err(DispatchError::NotMcpName)` when the name doesn't match the
    /// MCP shape (caller should fall back to native dispatch).
    pub async fn dispatch_tool(
        &self,
        advertised_name: &str,
        arguments: Value,
        timeout: std::time::Duration,
    ) -> Result<ToolCallOutcome, DispatchError> {
        let (server_name, tool_name) =
            protocol::split_advertised(advertised_name).ok_or(DispatchError::NotMcpName)?;
        let server = self
            .get(server_name)
            .await
            .ok_or_else(|| DispatchError::UnknownServer(server_name.to_owned()))?;
        if server.status != McpServerStatus::Connected {
            return Err(DispatchError::ServerNotConnected(server_name.to_owned()));
        }
        let Some(transport) = server.transport.as_ref() else {
            return Err(DispatchError::ServerNotConnected(server_name.to_owned()));
        };
        let result = transport
            .call_tool(tool_name, arguments, timeout)
            .await
            .map_err(DispatchError::Request)?;
        Ok(ToolCallOutcome::from(result))
    }
}

#[derive(Debug)]
pub enum DispatchError {
    /// The advertised name didn't start with `mcp__` — caller should
    /// dispatch through the native tool path.
    NotMcpName,
    /// Server name parsed from the advertised tool isn't in the
    /// registry (server crashed / never connected).
    UnknownServer(String),
    /// Server entry exists but transport is gone.
    ServerNotConnected(String),
    /// Lower-layer transport error.
    Request(RequestError),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotMcpName => f.write_str("tool name is not in mcp__server__tool format"),
            Self::UnknownServer(s) => write!(f, "unknown MCP server: {s}"),
            Self::ServerNotConnected(s) => write!(f, "MCP server {s} is not connected"),
            Self::Request(e) => write!(f, "MCP dispatch error: {e}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Spawn a single server from a config block, run the handshake +
/// `tools/list`, and return the resulting [`McpServer`] entry. On any
/// failure returns a `Failed` entry (no transport) so `/mcp list` can
/// still surface the configured name.
pub async fn build_server(name: &str, cfg: &McpServerConfig) -> McpServer {
    // `type` is authoritative; resolution validates that the required
    // fields (`command` for stdio, `url` for http) are present.
    let kind = match cfg.resolve_transport() {
        Ok(k) => k,
        Err(reason) => {
            tracing::warn!(
                target: "jfc::mcp",
                server = %name,
                reason = %reason,
                "invalid MCP server config — marking failed"
            );
            return McpServer {
                name: name.to_owned(),
                status: McpServerStatus::Failed,
                tools: Vec::new(),
                resources: Vec::new(),
                transport: None,
                spawn_cfg: SpawnConfig {
                    server_name: name.to_owned(),
                    kind: TransportKind::Stdio,
                    command: cfg.command.clone().unwrap_or_default(),
                    args: cfg.args.clone(),
                    env: cfg.env.clone(),
                    headers: cfg.headers.clone(),
                    url: cfg.url.clone(),
                },
            };
        }
    };

    let spawn_cfg = SpawnConfig {
        server_name: name.to_owned(),
        kind,
        command: cfg.command.clone().unwrap_or_default(),
        args: cfg.args.clone(),
        env: cfg.env.clone(),
        headers: cfg.headers.clone(),
        url: cfg.url.clone(),
    };

    let Some(transport) = Transport::spawn(spawn_cfg.clone()).await else {
        return McpServer {
            name: name.to_owned(),
            status: McpServerStatus::Failed,
            tools: Vec::new(),
            resources: Vec::new(),
            transport: None,
            spawn_cfg,
        };
    };

    // Discover tools.
    let tools = fetch_all_tools(&transport).await;
    tracing::info!(
        target: "jfc::mcp",
        server = %name,
        tool_count = tools.len(),
        "mcp server registered"
    );
    McpServer {
        name: name.to_owned(),
        status: McpServerStatus::Connected,
        tools,
        resources: Vec::new(),
        transport: Some(transport),
        spawn_cfg,
    }
}

/// Discover every tool the server exposes. `rmcp`'s `list_all_tools`
/// walks `tools/list` cursor pagination internally, so a server with
/// hundreds of tools across multiple pages isn't truncated. On error we
/// return an empty list — a server that can't be enumerated shouldn't
/// crash startup.
async fn fetch_all_tools(transport: &Transport) -> Vec<McpTool> {
    match transport.list_tools().await {
        Ok(tools) => tools.into_iter().map(McpTool::from).collect(),
        Err(e) => {
            tracing::warn!(
                target: "jfc::mcp",
                server = %transport.server_name(),
                error = %e,
                "tools/list failed"
            );
            Vec::new()
        }
    }
}

/// Spawn every `[mcp.<name>]` entry from a config and insert them into
/// the registry. Failures are logged and a `Failed` entry is added so
/// `/mcp list` reflects the configured set.
pub async fn register_servers_from_config(
    registry: &McpRegistry,
    configs: &HashMap<String, McpServerConfig>,
) {
    if configs.is_empty() {
        return;
    }
    if matches!(
        std::env::var("JFC_DISABLE_MCP").as_deref(),
        Ok("1") | Ok("true")
    ) {
        tracing::info!(target: "jfc::mcp", "MCP disabled via JFC_DISABLE_MCP");
        return;
    }
    for (name, cfg) in configs {
        let server = build_server(name, cfg).await;
        registry.insert(server).await;
    }
}

/// Restart a server by name: removes the current entry (dropping it so
/// the child process is killed), then runs the spawn flow again with
/// the cached `spawn_cfg`. Returns the new status — `Some(true)` when
/// reconnected, `Some(false)` when the new spawn also failed, `None`
/// when no entry by that name exists.
pub async fn restart_server(registry: &McpRegistry, name: &str) -> Option<bool> {
    let old = registry.remove(name).await?;
    // Try to clean shutdown the old transport before rebuild.
    if let Some(t) = old.transport.as_ref() {
        t.shutdown().await;
    }
    // Reconstruct McpServerConfig from cached spawn cfg, preserving the
    // resolved transport kind so a restart can't silently switch
    // transports.
    let cfg = McpServerConfig {
        server_type: Some(old.spawn_cfg.kind.label().to_owned()),
        command: Some(old.spawn_cfg.command.clone()),
        args: old.spawn_cfg.args.clone(),
        env: old.spawn_cfg.env.clone(),
        headers: old.spawn_cfg.headers.clone(),
        url: old.spawn_cfg.url.clone(),
    };
    let new_server = build_server(name, &cfg).await;
    let connected = new_server.status == McpServerStatus::Connected;
    registry.insert(new_server).await;
    Some(connected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fake_server(name: &str, status: McpServerStatus, tools: Vec<McpTool>) -> McpServer {
        McpServer {
            name: name.to_owned(),
            status,
            tools,
            resources: Vec::new(),
            transport: None,
            spawn_cfg: SpawnConfig {
                server_name: name.to_owned(),
                kind: TransportKind::Stdio,
                command: "fake".into(),
                args: Vec::new(),
                env: HashMap::new(),
                headers: HashMap::new(),
                url: None,
            },
        }
    }

    #[tokio::test]
    async fn registry_insert_and_get_normal() {
        let reg = McpRegistry::new();
        reg.insert(fake_server(
            "fs",
            McpServerStatus::Connected,
            vec![McpTool {
                name: "read".into(),
                description: "Read".into(),
                input_schema: json!({"type":"object"}),
            }],
        ))
        .await;
        let got = reg.get("fs").await.unwrap();
        assert_eq!(got.name, "fs");
        assert_eq!(got.tools.len(), 1);
    }

    #[tokio::test]
    async fn registry_remove_normal() {
        let reg = McpRegistry::new();
        reg.insert(fake_server("fs", McpServerStatus::Connected, vec![]))
            .await;
        assert!(reg.get("fs").await.is_some());
        reg.remove("fs").await;
        assert!(reg.get("fs").await.is_none());
    }

    #[tokio::test]
    async fn registry_remove_missing_is_none_robust() {
        let reg = McpRegistry::new();
        assert!(reg.remove("ghost").await.is_none());
    }

    #[tokio::test]
    async fn registry_list_returns_all_normal() {
        let reg = McpRegistry::new();
        reg.insert(fake_server("a", McpServerStatus::Connected, vec![]))
            .await;
        reg.insert(fake_server("b", McpServerStatus::Failed, vec![]))
            .await;
        let mut names: Vec<String> = reg.list().await.iter().map(|s| s.name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn list_active_filters_failed_robust() {
        let reg = McpRegistry::new();
        reg.insert(fake_server("good", McpServerStatus::Connected, vec![]))
            .await;
        reg.insert(fake_server("bad", McpServerStatus::Failed, vec![]))
            .await;
        // None have transports → none active even though one is "Connected".
        let active = reg.list_active().await;
        assert!(
            active.is_empty(),
            "list_active requires both Connected status AND Some(transport)"
        );
    }

    #[tokio::test]
    async fn advertised_tool_defs_namespace_normal() {
        let s = fake_server(
            "git",
            McpServerStatus::Connected,
            vec![McpTool {
                name: "status".into(),
                description: "Show status".into(),
                input_schema: json!({"type":"object"}),
            }],
        );
        let defs = s.advertised_tool_defs();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "mcp__git__status");
        assert_eq!(defs[0].description, "Show status");
    }

    #[tokio::test]
    async fn dispatch_rejects_non_mcp_name_robust() {
        let reg = McpRegistry::new();
        let res = reg
            .dispatch_tool("Bash", json!({}), std::time::Duration::from_secs(1))
            .await;
        assert!(matches!(res, Err(DispatchError::NotMcpName)));
    }

    #[tokio::test]
    async fn dispatch_unknown_server_robust() {
        let reg = McpRegistry::new();
        let res = reg
            .dispatch_tool(
                "mcp__missing__do_thing",
                json!({}),
                std::time::Duration::from_secs(1),
            )
            .await;
        assert!(matches!(res, Err(DispatchError::UnknownServer(s)) if s == "missing"));
    }

    #[tokio::test]
    async fn dispatch_server_without_transport_robust() {
        let reg = McpRegistry::new();
        reg.insert(fake_server("brokes", McpServerStatus::Failed, vec![]))
            .await;
        let res = reg
            .dispatch_tool(
                "mcp__brokes__do_thing",
                json!({}),
                std::time::Duration::from_secs(1),
            )
            .await;
        assert!(matches!(res, Err(DispatchError::ServerNotConnected(s)) if s == "brokes"));
    }

    #[tokio::test]
    async fn register_servers_from_empty_config_normal() {
        let reg = McpRegistry::new();
        let configs: HashMap<String, McpServerConfig> = HashMap::new();
        register_servers_from_config(&reg, &configs).await;
        assert!(reg.list().await.is_empty());
    }

    #[tokio::test]
    async fn register_servers_with_missing_command_marks_failed_robust() {
        let reg = McpRegistry::new();
        let mut configs = HashMap::new();
        configs.insert(
            "noop".into(),
            McpServerConfig {
                server_type: Some("stdio".into()),
                command: None,
                args: vec![],
                env: HashMap::new(),
                headers: HashMap::new(),
                url: None,
            },
        );
        register_servers_from_config(&reg, &configs).await;
        let entry = reg.get("noop").await.unwrap();
        assert_eq!(entry.status, McpServerStatus::Failed);
        assert!(entry.transport.is_none());
    }

    #[tokio::test]
    async fn register_servers_with_bad_binary_marks_failed_robust() {
        // Arbitrary path that almost certainly doesn't exist on PATH.
        // Spawn fails → Failed entry.
        let reg = McpRegistry::new();
        let mut configs = HashMap::new();
        configs.insert(
            "ghost".into(),
            McpServerConfig {
                server_type: Some("stdio".into()),
                command: Some("/nonexistent/jfc-mcp-test-binary".into()),
                args: vec![],
                env: HashMap::new(),
                headers: HashMap::new(),
                url: None,
            },
        );
        register_servers_from_config(&reg, &configs).await;
        let entry = reg.get("ghost").await.unwrap();
        assert_eq!(entry.status, McpServerStatus::Failed);
    }

    #[tokio::test]
    async fn all_advertised_tool_defs_aggregates_normal() {
        let reg = McpRegistry::new();
        // Manually flag one as Connected with a transport-less stub —
        // list_active filters by transport so this won't show.
        let mut s = fake_server(
            "fs",
            McpServerStatus::Connected,
            vec![McpTool {
                name: "read".into(),
                description: "".into(),
                input_schema: json!({"type":"object"}),
            }],
        );
        s.transport = None;
        reg.insert(s).await;
        let defs = reg.all_advertised_tool_defs().await;
        assert!(
            defs.is_empty(),
            "transport-less Connected entries are excluded from active list"
        );
    }
}
