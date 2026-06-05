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

use reqwest::header::{HeaderName, HeaderValue};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientInfo, Implementation,
    ReadResourceRequestParams, ReadResourceResult, Resource, Tool,
};
use rmcp::service::{NotificationContext, RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
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
    /// HTTP auth was configured explicitly and the server rejected it.
    AuthHeaderRejected,
    /// Network/transport-level MCP failure. Kept distinct from model/tool
    /// errors so callers can present it as infrastructure trouble.
    Transport { code: &'static str, message: String },
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
            Self::AuthHeaderRejected => f.write_str(
                "MCP server rejected the configured Authorization header; \
                 check the token for this endpoint",
            ),
            Self::Transport { code, message } => {
                write!(f, "MCP transport error ({code}): {message}")
            }
            Self::BadArguments => f.write_str("MCP tool arguments must be a JSON object"),
            Self::Service(m) => write!(f, "MCP service error: {m}"),
        }
    }
}

impl std::error::Error for RequestError {}

impl From<ServiceError> for RequestError {
    fn from(e: ServiceError) -> Self {
        map_service_error(e, false)
    }
}

fn map_service_error(e: ServiceError, has_auth_header: bool) -> RequestError {
    match e {
        ServiceError::TransportClosed | ServiceError::Cancelled { .. } => {
            RequestError::Disconnected
        }
        ServiceError::Timeout { .. } => RequestError::Timeout,
        ServiceError::TransportSend(e) => {
            classify_transport_error(e.error.as_ref(), has_auth_header)
        }
        other => RequestError::Service(other.to_string()),
    }
}

fn classify_transport_error(
    err: &(dyn std::error::Error + Send + Sync + 'static),
    has_auth_header: bool,
) -> RequestError {
    let message = err.to_string();
    let lower = message.to_ascii_lowercase();
    if has_auth_header
        && (lower.contains("401")
            || lower.contains("403")
            || lower.contains("unauthorized")
            || lower.contains("forbidden"))
    {
        return RequestError::AuthHeaderRejected;
    }
    let code = if lower.contains("timeout") || lower.contains("timed out") {
        "timeout"
    } else if lower.contains("connection refused") || lower.contains("econnrefused") {
        "connection_refused"
    } else if lower.contains("connection reset") || lower.contains("econnreset") {
        "connection_reset"
    } else if lower.contains("dns") || lower.contains("enotfound") || lower.contains("eai_again") {
        "dns"
    } else if lower.contains("closed") || lower.contains("terminated") {
        "connection_closed"
    } else {
        "transport"
    };
    RequestError::Transport { code, message }
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
    pub headers: HashMap<String, String>,
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
    has_auth_header: bool,
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
                has_auth_header: false,
                stderr_ring,
            }),
        })
    }

    async fn spawn_http(cfg: SpawnConfig) -> Option<Self> {
        let url = cfg.url.as_deref().unwrap_or_default();
        let (headers, has_auth_header) = match header_map_from_config(&cfg.headers) {
            Ok(headers) => (
                headers,
                cfg.headers
                    .keys()
                    .any(|key| key.eq_ignore_ascii_case("authorization")),
            ),
            Err(e) => {
                tracing::warn!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    error = %e,
                    "invalid MCP HTTP headers"
                );
                return None;
            }
        };
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers),
        );
        let client = match JfcClientHandler.serve(transport).await {
            Ok(c) => c,
            Err(e) => {
                let rejected_auth = has_auth_header && service_error_suggests_auth_rejection(&e);
                if rejected_auth {
                    tracing::warn!(
                        target: "jfc::mcp",
                        server = %cfg.server_name,
                        url = %url,
                        error = ?e,
                        "mcp http initialize rejected configured Authorization header"
                    );
                } else {
                    tracing::warn!(
                        target: "jfc::mcp",
                        server = %cfg.server_name,
                        url = %url,
                        error = ?e,
                        "mcp http initialize handshake failed"
                    );
                }
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
                has_auth_header,
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
            .map_err(|e| map_service_error(e, self.inner.has_auth_header))
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
            Ok(Err(e)) => Err(map_service_error(e, self.inner.has_auth_header)),
            Err(_) => Err(RequestError::Timeout),
        }
    }

    /// Snapshot of the most recent stderr lines (most recent last). Empty
    /// for HTTP transports.
    pub async fn recent_stderr(&self) -> Vec<String> {
        let guard = self.inner.stderr_ring.lock().await;
        guard.iter().cloned().collect()
    }

    /// Raw JSON-RPC escape hatch. The `rmcp` SDK does not expose a generic
    /// "send this arbitrary JSON-RPC frame" entry point — every supported
    /// MCP method has a typed wrapper (`call_tool`, [`Self::read_resource`],
    /// [`Self::list_resources`], …). Callers should use those. This method
    /// returns an explicit unsupported error rather than silently timing out
    /// so a future caller gets an actionable message instead of a 30s hang.
    pub async fn request(
        &self,
        request: serde_json::Value,
        _timeout: std::time::Duration,
    ) -> Result<serde_json::Value, RequestError> {
        let method = request
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("<unknown>");
        Err(RequestError::Service(format!(
            "raw JSON-RPC dispatch is not supported by the rmcp transport \
             (method `{method}`); use the typed call_tool / read_resource / \
             list_resources wrappers instead"
        )))
    }

    /// List all resources advertised by this server.
    pub async fn list_resources(&self) -> Result<Vec<Resource>, RequestError> {
        self.inner
            .client
            .list_all_resources()
            .await
            .map_err(|e| map_service_error(e, self.inner.has_auth_header))
    }

    /// Read a resource by URI from this server.
    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, RequestError> {
        let params = ReadResourceRequestParams::new(uri);
        self.inner
            .client
            .read_resource(params)
            .await
            .map_err(|e| map_service_error(e, self.inner.has_auth_header))
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

fn header_map_from_config(
    headers: &HashMap<String, String>,
) -> Result<HashMap<HeaderName, HeaderValue>, String> {
    let mut out = HashMap::with_capacity(headers.len());
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| format!("invalid header name `{name}`: {e}"))?;
        let value = HeaderValue::from_str(value)
            .map_err(|e| format!("invalid value for header `{name}`: {e}"))?;
        out.insert(name, value);
    }
    Ok(out)
}

fn service_error_suggests_auth_rejection(error: &impl std::fmt::Display) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("401")
        || message.contains("403")
        || message.contains("unauthorized")
        || message.contains("forbidden")
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

    #[derive(Debug)]
    struct StringError(&'static str);

    impl std::fmt::Display for StringError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for StringError {}

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

    #[test]
    fn header_map_from_config_accepts_custom_headers_normal() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_owned(), "Bearer token".to_owned());
        headers.insert("X-Test".to_owned(), "ok".to_owned());
        let map = header_map_from_config(&headers).unwrap();
        assert_eq!(
            map.get(&HeaderName::from_static("authorization")).unwrap(),
            &HeaderValue::from_static("Bearer token")
        );
        assert_eq!(
            map.get(&HeaderName::from_static("x-test")).unwrap(),
            &HeaderValue::from_static("ok")
        );
    }

    #[test]
    fn header_map_from_config_rejects_bad_headers_robust() {
        let mut headers = HashMap::new();
        headers.insert("bad header".to_owned(), "ok".to_owned());
        assert!(header_map_from_config(&headers).is_err());

        let mut headers = HashMap::new();
        headers.insert("x-test".to_owned(), "bad\nvalue".to_owned());
        assert!(header_map_from_config(&headers).is_err());
    }

    #[test]
    fn auth_rejection_requires_configured_auth_header_robust() {
        let err = StringError("server returned 401 unauthorized");
        assert!(matches!(
            classify_transport_error(&err, true),
            RequestError::AuthHeaderRejected
        ));
        assert!(matches!(
            classify_transport_error(&err, false),
            RequestError::Transport {
                code: "transport",
                ..
            }
        ));
    }
}
