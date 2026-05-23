//! Transport to a single MCP server, backed by the [`rmcp`] SDK.
//!
//! `rmcp` owns everything that used to be hand-rolled here: JSON-RPC 2.0
//! framing, the `initialize` / `notifications/initialized` handshake,
//! pending-request routing, and `tools/list` pagination. This module is
//! a thin wrapper that:
//!
//! 1. Spawns the right `rmcp` transport for the configured server —
//!    [`TokioChildProcess`] for stdio (`command`/`args`/`env`) or
//!    [`StreamableHttpClientTransport`] for a remote `url`.
//! 2. Keeps a bounded ring buffer of the child's stderr lines so
//!    `/mcp logs <name>` can show the user what blew up (stdio only —
//!    HTTP servers have no local stderr).
//! 3. Routes `notifications/tools/list_changed` to the process-global
//!    refresh signal via a custom [`ClientHandler`].
//!
//! The live `rmcp` client handle ([`RunningService`]) is held behind an
//! `Arc` so cloned [`Transport`] handles share one connection. Dropping
//! the last handle cancels the service task and kills the child process
//! (rmcp's `TokioChildProcess` kills on drop).

use std::collections::HashMap;
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientInfo, Implementation, Tool,
};
use rmcp::service::{NotificationContext, RoleClient, RunningService};
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{ClientHandler, ServiceError, ServiceExt};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

/// Maximum number of stderr lines to keep in the in-memory ring buffer
/// per server. 200 is generous for a `/mcp logs` printout — most npm
/// packages emit fewer than that across their entire lifetime.
const DEFAULT_STDERR_RING_CAPACITY: usize = 200;

type StderrRing = Arc<Mutex<VecDeque<String>>>;

/// `rmcp` client handler. Reports jfc's identity in the `initialize`
/// handshake and forwards `tools/list_changed` notifications to the
/// process-global refresh signal so the streaming layer re-reads the
/// catalog. All other client-role callbacks keep their no-op defaults.
#[derive(Clone)]
struct JfcClientHandler;

impl ClientHandler for JfcClientHandler {
    async fn on_tool_list_changed(&self, _ctx: NotificationContext<RoleClient>) {
        crate::registry::request_refresh();
        tracing::info!(
            target: "jfc::mcp",
            "received notifications/tools/list_changed — registry refresh requested"
        );
    }

    fn get_info(&self) -> ClientInfo {
        ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("jfc", env!("CARGO_PKG_VERSION")),
        )
    }
}

/// Transport-layer failures surfaced to the dispatcher. Wraps the
/// underlying [`rmcp::ServiceError`] but collapses the two cases the
/// registry treats specially.
#[derive(Debug)]
pub enum RequestError {
    /// The connection to the server is gone (transport closed / task
    /// cancelled).
    Disconnected,
    /// No response within the dispatch deadline.
    Timeout,
    /// The model produced arguments that weren't a JSON object.
    BadArguments,
    /// Any other `rmcp` service error (server-side MCP error, unexpected
    /// response, transport send failure).
    Service(String),
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => f.write_str("MCP server disconnected"),
            Self::Timeout => f.write_str("MCP request timed out"),
            Self::BadArguments => f.write_str("MCP tool arguments must be a JSON object"),
            Self::Service(m) => write!(f, "MCP service error: {m}"),
        }
    }
}

impl std::error::Error for RequestError {}

impl From<ServiceError> for RequestError {
    fn from(e: ServiceError) -> Self {
        match e {
            ServiceError::TransportClosed | ServiceError::Cancelled { .. } => Self::Disconnected,
            ServiceError::Timeout { .. } => Self::Timeout,
            other => Self::Service(other.to_string()),
        }
    }
}

/// Which rmcp client transport to drive for a server. Resolved
/// authoritatively from the `type` config field (see
/// [`crate::McpServerConfig::resolve_transport`]); jfc only infers it
/// from `command`/`url` presence when `type` is omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// Local child process speaking JSON-RPC over stdio
    /// ([`TokioChildProcess`]). Driven by `command`/`args`/`env`.
    Stdio,
    /// Remote server over the streamable-HTTP transport
    /// ([`StreamableHttpClientTransport`]). Driven by `url`.
    Http,
}

impl TransportKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }
}

/// Configuration for spawning a transport. `kind` selects the rmcp
/// transport; `command`/`args`/`env` feed [`TransportKind::Stdio`] and
/// `url` feeds [`TransportKind::Http`].
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub server_name: String,
    pub kind: TransportKind,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub url: Option<String>,
}

/// A live connection to one MCP server. Cloneable handles share the same
/// underlying `rmcp` client.
#[derive(Clone)]
pub struct Transport {
    inner: Arc<TransportInner>,
}

struct TransportInner {
    server_name: String,
    client: RunningService<RoleClient, JfcClientHandler>,
    stderr_ring: StderrRing,
}

impl Transport {
    /// Server display name (the key from `[mcp.<name>]` in the config).
    pub fn server_name(&self) -> &str {
        &self.inner.server_name
    }

    /// Spawn the transport selected by `cfg.kind` and run the `rmcp`
    /// handshake. Returns `None` on any failure (binary missing,
    /// handshake timeout, bad URL) so callers can keep going without that
    /// server — same silent-fallthrough policy as `lsp_client.rs`.
    pub async fn spawn(cfg: SpawnConfig) -> Option<Self> {
        match cfg.kind {
            TransportKind::Stdio => Self::spawn_stdio(cfg).await,
            TransportKind::Http => Self::spawn_http(cfg).await,
        }
    }

    async fn spawn_stdio(cfg: SpawnConfig) -> Option<Self> {
        let mut command = Command::new(&cfg.command);
        command.args(&cfg.args);
        for (k, v) in &cfg.env {
            command.env(k, v);
        }

        // Pipe stderr so we can surface it via `/mcp logs`; rmcp's builder
        // sets stdin/stdout/stderr itself, so we only configure args/env
        // on the raw command above.
        let (proc, stderr) = match TokioChildProcess::builder(command)
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(pair) => pair,
            Err(e) => {
                tracing::info!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    command = %cfg.command,
                    error = %e,
                    "spawn failed (binary likely not on PATH)"
                );
                return None;
            }
        };

        let stderr_ring: StderrRing = Arc::new(Mutex::new(VecDeque::with_capacity(
            DEFAULT_STDERR_RING_CAPACITY,
        )));
        if let Some(stderr) = stderr {
            spawn_stderr_drain(cfg.server_name.clone(), stderr, Arc::clone(&stderr_ring));
        }

        let client = match JfcClientHandler.serve(proc).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    error = ?e,
                    "mcp initialize handshake failed"
                );
                return None;
            }
        };

        tracing::info!(
            target: "jfc::mcp",
            server = %cfg.server_name,
            command = %cfg.command,
            "mcp stdio transport ready"
        );
        Some(Self {
            inner: Arc::new(TransportInner {
                server_name: cfg.server_name,
                client,
                stderr_ring,
            }),
        })
    }

    async fn spawn_http(cfg: SpawnConfig) -> Option<Self> {
        let url = cfg.url.as_deref().unwrap_or_default();
        let transport = StreamableHttpClientTransport::from_uri(url);
        let client = match JfcClientHandler.serve(transport).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    url = %url,
                    error = ?e,
                    "mcp http initialize handshake failed"
                );
                return None;
            }
        };

        tracing::info!(
            target: "jfc::mcp",
            server = %cfg.server_name,
            url = %url,
            "mcp http transport ready"
        );
        Some(Self {
            inner: Arc::new(TransportInner {
                server_name: cfg.server_name,
                client,
                // HTTP servers have no local stderr to capture.
                stderr_ring: Arc::new(Mutex::new(VecDeque::new())),
            }),
        })
    }

    /// List every tool the server exposes. `rmcp` walks `tools/list`
    /// pagination internally.
    pub async fn list_tools(&self) -> Result<Vec<Tool>, RequestError> {
        self.inner
            .client
            .list_all_tools()
            .await
            .map_err(RequestError::from)
    }

    /// Invoke `tool_name` with `arguments`, bounded by `timeout`.
    /// `arguments` must be a JSON object (or null for no-arg tools).
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<CallToolResult, RequestError> {
        let args = match arguments {
            Value::Object(map) => Some(map),
            Value::Null => None,
            _ => return Err(RequestError::BadArguments),
        };
        let mut params = CallToolRequestParams::new(tool_name.to_owned());
        if let Some(map) = args {
            params = params.with_arguments(map);
        }

        match tokio::time::timeout(timeout, self.inner.client.call_tool(params)).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(e)) => Err(RequestError::from(e)),
            Err(_) => Err(RequestError::Timeout),
        }
    }

    /// Snapshot of the most recent stderr lines (most recent last). Empty
    /// for HTTP transports.
    pub async fn recent_stderr(&self) -> Vec<String> {
        let guard = self.inner.stderr_ring.lock().await;
        guard.iter().cloned().collect()
    }

    /// Best-effort shutdown. `rmcp` cancels the service task and kills the
    /// child process when the last [`RunningService`] handle is dropped
    /// (which happens when the registry entry is removed), so there is no
    /// explicit teardown to perform from behind the shared `Arc`.
    pub async fn shutdown(&self) {
        tracing::debug!(
            target: "jfc::mcp",
            server = %self.inner.server_name,
            "shutdown requested — teardown happens on drop"
        );
    }
}

/// Drain a child's stderr line-by-line into `tracing` and a bounded ring
/// buffer for `/mcp logs`.
fn spawn_stderr_drain(server_name: String, stderr: tokio::process::ChildStderr, ring: StderrRing) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            tracing::debug!(
                target: "jfc::mcp",
                server = %server_name,
                stderr = %line,
                "mcp stderr"
            );
            let mut guard = ring.lock().await;
            if guard.len() == DEFAULT_STDERR_RING_CAPACITY {
                guard.pop_front();
            }
            guard.push_back(line);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_error_maps_transport_closed_to_disconnected() {
        let mapped = RequestError::from(ServiceError::TransportClosed);
        assert!(matches!(mapped, RequestError::Disconnected));
    }

    #[test]
    fn request_error_maps_timeout() {
        let mapped = RequestError::from(ServiceError::Timeout {
            timeout: Duration::from_secs(1),
        });
        assert!(matches!(mapped, RequestError::Timeout));
    }

    #[test]
    fn transport_kind_label() {
        assert_eq!(TransportKind::Stdio.label(), "stdio");
        assert_eq!(TransportKind::Http.label(), "http");
    }
}
