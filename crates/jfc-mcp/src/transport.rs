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

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ReadResourceRequestParams, ReadResourceResult, Resource,
    Tool,
};
use rmcp::service::{RoleClient, RunningService};
use serde_json::Value;
use tokio::sync::Mutex;

mod config;
mod error;
mod handler;
mod http;
mod spawn;
mod stderr;
mod trace;

pub use config::{SpawnConfig, TransportKind};
pub use error::RequestError;
use error::map_service_error;
use handler::JfcClientHandler;

type StderrRing = Arc<Mutex<std::collections::VecDeque<String>>>;

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

    /// Instructions returned by the server in the MCP `initialize` result.
    /// Hosts are expected to surface these to the model so servers can teach
    /// their own tool-selection rules without writing AGENTS.md/CLAUDE.md.
    pub fn server_instructions(&self) -> Option<String> {
        self.inner
            .client
            .peer()
            .peer_info()
            .and_then(|info| info.instructions.clone())
            .map(|instructions| instructions.trim().to_owned())
            .filter(|instructions| !instructions.is_empty())
    }

    /// List every tool the server exposes. `rmcp` walks `tools/list`
    /// pagination internally.
    pub async fn list_tools(&self) -> Result<Vec<Tool>, RequestError> {
        let _linkscope_list = linkscope::phase("mcp.list_tools");
        trace::list_start(&self.inner.server_name, "tools/list");
        let result = self
            .inner
            .client
            .list_all_tools()
            .await
            .map_err(|e| map_service_error(e, self.inner.has_auth_header));
        if let Ok(tools) = &result {
            linkscope::record_items("mcp.tools.listed", usize_to_u64_saturating(tools.len()));
            trace::list_result(trace::ListResult {
                server: &self.inner.server_name,
                method: "tools/list",
                status: "ok",
                count: tools.len(),
            });
        } else {
            trace::list_result(trace::ListResult {
                server: &self.inner.server_name,
                method: "tools/list",
                status: "error",
                count: 0,
            });
        }
        result
    }

    /// Invoke `tool_name` with `arguments`, bounded by `timeout`.
    /// `arguments` must be a JSON object (or null for no-arg tools).
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<CallToolResult, RequestError> {
        let _linkscope_call = linkscope::phase("mcp.call_tool");
        trace::call_tool_start(trace::CallToolStart {
            server: &self.inner.server_name,
            tool_name,
            timeout,
            args: &arguments,
        });
        let args = match arguments {
            Value::Object(map) => Some(map),
            Value::Null => None,
            _ => {
                trace::call_tool_result(&self.inner.server_name, tool_name, "bad_arguments");
                return Err(RequestError::BadArguments);
            }
        };
        let mut params = CallToolRequestParams::new(tool_name.to_owned());
        if let Some(map) = args {
            params = params.with_arguments(map);
        }

        match tokio::time::timeout(timeout, self.inner.client.call_tool(params)).await {
            Ok(Ok(result)) => {
                linkscope::record_items("mcp.call_tool.ok", 1);
                trace::call_tool_result(&self.inner.server_name, tool_name, "ok");
                Ok(result)
            }
            Ok(Err(e)) => {
                linkscope::record_items("mcp.call_tool.error", 1);
                trace::call_tool_result(&self.inner.server_name, tool_name, "error");
                Err(map_service_error(e, self.inner.has_auth_header))
            }
            Err(_) => {
                linkscope::record_items("mcp.call_tool.timeout", 1);
                trace::call_tool_result(&self.inner.server_name, tool_name, "timeout");
                Err(RequestError::Timeout)
            }
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
        let _linkscope_list = linkscope::phase("mcp.list_resources");
        trace::list_start(&self.inner.server_name, "resources/list");
        let result = self
            .inner
            .client
            .list_all_resources()
            .await
            .map_err(|e| map_service_error(e, self.inner.has_auth_header));
        if let Ok(resources) = &result {
            linkscope::record_items(
                "mcp.resources.listed",
                usize_to_u64_saturating(resources.len()),
            );
            trace::list_result(trace::ListResult {
                server: &self.inner.server_name,
                method: "resources/list",
                status: "ok",
                count: resources.len(),
            });
        } else {
            trace::list_result(trace::ListResult {
                server: &self.inner.server_name,
                method: "resources/list",
                status: "error",
                count: 0,
            });
        }
        result
    }

    /// Read a resource by URI from this server.
    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, RequestError> {
        let _linkscope_read = linkscope::phase("mcp.read_resource");
        trace::read_resource_start(&self.inner.server_name, uri);
        let params = ReadResourceRequestParams::new(uri);
        let result = self
            .inner
            .client
            .read_resource(params)
            .await
            .map_err(|e| map_service_error(e, self.inner.has_auth_header));
        trace::read_resource_result(
            &self.inner.server_name,
            if result.is_ok() { "ok" } else { "error" },
        );
        result
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

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
