//! JSON-RPC 2.0 + MCP message shapes.
//!
//! MCP (Model Context Protocol) rides on top of JSON-RPC 2.0 with the
//! same `Content-Length: N\r\n\r\n{json}` framing as LSP. The framing
//! itself lives in [`super::transport`]; this module owns the wire
//! shapes for the handshake, tools/list, tools/call, and notifications.
//!
//! ## What MCP defines on top of plain JSON-RPC
//!
//! - `initialize` — first request a client sends. Carries
//!   `protocolVersion`, `capabilities`, and a `clientInfo` block.
//!   Server reply mirrors the structure.
//! - `notifications/initialized` — fire-and-forget after `initialize`'s
//!   response. Until the server sees this it MAY refuse to expose tools.
//! - `tools/list` — paginated list of available tools. Each entry is
//!   `{name, description, inputSchema}` — same shape as
//!   `provider::ToolDef`.
//! - `tools/call` — invoke a tool by name with a JSON arguments object.
//!   Result is `{content: [{type: "text", text: ...}], isError: bool}`.
//! - `notifications/tools/list_changed` — server pushes when its tool
//!   set changes; client should re-issue `tools/list`.
//!
//! ## Why we use raw `serde_json::Value` for params/results
//!
//! MCP servers ship arbitrary `inputSchema` JSON. Strongly typing every
//! request/result would force conversion at every boundary with no real
//! win — same call we made for [`crate::lsp_rpc`]. We lean on
//! `serde_json::json!` macros and pull-out helpers below.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Latest MCP protocol version we negotiate against. Real servers will
/// often accept older versions too, but advertising the current one is
/// the polite default.
pub const PROTOCOL_VERSION: &str = "2025-03-26";

/// JSON-RPC 2.0 request envelope. MCP `id`s are integers in practice,
/// but the spec also allows strings; we keep an integer for now since
/// our writer only mints integers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 notification envelope (no `id`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 response envelope. Either `result` or `error` is set,
/// never both.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Build the `initialize` request body. `client_name` and `client_version`
/// land in `clientInfo` so servers can log who connected.
pub fn build_initialize(id: u64, client_name: &str, client_version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": {},
                "roots": { "listChanged": false },
                "sampling": {}
            },
            "clientInfo": {
                "name": client_name,
                "version": client_version
            }
        }
    })
}

/// Build the `notifications/initialized` notification — sent after the
/// `initialize` response arrives. Servers may withhold tool listings
/// until they see this.
pub fn build_initialized_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    })
}

/// Build a `tools/list` request. MCP supports cursor-based pagination
/// via `params.cursor`; pass `None` for the first page.
pub fn build_tools_list(id: u64, cursor: Option<&str>) -> Value {
    let mut params = serde_json::Map::new();
    if let Some(c) = cursor {
        params.insert("cursor".into(), json!(c));
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": Value::Object(params)
    })
}

/// Build a `tools/call` request. `arguments` is the JSON object the
/// model produced; servers validate against the tool's `inputSchema`.
pub fn build_tools_call(id: u64, tool_name: &str, arguments: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    })
}

/// One entry from a `tools/list` response. Mirrors the fields jfc cares
/// about — servers may include richer metadata (annotations, etc.) which
/// we just ignore here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_input_schema", rename = "inputSchema")]
    pub input_schema: Value,
}

fn default_input_schema() -> Value {
    json!({ "type": "object" })
}

/// Parse the `result` of a `tools/list` response into a Vec of tools.
/// Returns an empty vec when the result shape is unexpected — callers
/// shouldn't crash a UI loop because one server emitted something odd.
pub fn parse_tools_list_result(result: &Value) -> Vec<McpTool> {
    let Some(arr) = result.get("tools").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|t| serde_json::from_value::<McpTool>(t.clone()).ok())
        .collect()
}

/// Pull the textual content out of a `tools/call` result.
///
/// MCP returns `{content: [{type: "text", text: "..."}], isError: bool}`.
/// The model only cares about a flat string at this layer; we
/// concatenate every text-typed content block. Non-text content
/// (`{type: "image", data: "..."}` etc.) is currently dropped — TODO
/// follow-up: surface images as attachments.
pub fn parse_tools_call_result(result: &Value) -> ToolCallOutcome {
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut text = String::new();
    if let Some(arr) = result.get("content").and_then(|v| v.as_array()) {
        for block in arr {
            let kind = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "text"
                && let Some(s) = block.get("text").and_then(|v| v.as_str())
            {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(s);
            }
        }
    }
    ToolCallOutcome { text, is_error }
}

/// Result of a `tools/call`. `is_error` mirrors the MCP `isError` field
/// — true means the tool ran but reported a failure (the JSON-RPC layer
/// itself succeeded). JSON-RPC-level errors come back as
/// `RpcResponse.error` and are handled by the dispatcher.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallOutcome {
    pub text: String,
    pub is_error: bool,
}

/// Construct the canonical `mcp__<server>__<tool>` advertised name. This
/// is the format Anthropic's MCP spec uses to namespace tool names so a
/// `read_file` tool from `filesystem` and one from `git` don't collide.
pub fn advertise_tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// Inverse of [`advertise_tool_name`]. Returns `(server, tool)` when
/// `name` matches the `mcp__server__tool` shape, else `None`.
///
/// We split on `__` from the front so a tool name that contains `__`
/// internally (rare but legal) lands on the right side intact.
pub fn split_advertised(name: &str) -> Option<(&str, &str)> {
    let stripped = name.strip_prefix("mcp__")?;
    // Walk left→right, find the first `__` separator. Everything before
    // is the server, everything after is the (possibly multi-segment)
    // tool name.
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'_' && bytes[i + 1] == b'_' {
            let server = &stripped[..i];
            let tool = &stripped[i + 2..];
            if !server.is_empty() && !tool.is_empty() {
                return Some((server, tool));
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_initialize_has_required_fields_normal() {
        let req = build_initialize(1, "jfc", "0.1");
        assert_eq!(req["jsonrpc"], "2.0");
        assert_eq!(req["id"], 1);
        assert_eq!(req["method"], "initialize");
        assert_eq!(req["params"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(req["params"]["clientInfo"]["name"], "jfc");
        assert_eq!(req["params"]["clientInfo"]["version"], "0.1");
        assert!(req["params"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn build_initialized_is_notification_normal() {
        let n = build_initialized_notification();
        assert_eq!(n["jsonrpc"], "2.0");
        assert_eq!(n["method"], "notifications/initialized");
        assert!(n.get("id").is_none(), "notifications must not have id");
    }

    #[test]
    fn build_tools_list_no_cursor_normal() {
        let r = build_tools_list(7, None);
        assert_eq!(r["id"], 7);
        assert_eq!(r["method"], "tools/list");
        assert!(r["params"].is_object());
        assert!(r["params"].get("cursor").is_none());
    }

    #[test]
    fn build_tools_list_with_cursor_normal() {
        let r = build_tools_list(7, Some("page-2"));
        assert_eq!(r["params"]["cursor"], "page-2");
    }

    #[test]
    fn build_tools_call_shape_normal() {
        let args = json!({"path": "/tmp/x"});
        let r = build_tools_call(42, "read_file", &args);
        assert_eq!(r["id"], 42);
        assert_eq!(r["method"], "tools/call");
        assert_eq!(r["params"]["name"], "read_file");
        assert_eq!(r["params"]["arguments"], args);
    }

    #[test]
    fn parse_tools_list_result_normal() {
        let result = json!({
            "tools": [
                {
                    "name": "read_file",
                    "description": "Read a file",
                    "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}}
                },
                {
                    "name": "write_file",
                    "inputSchema": {"type": "object"}
                }
            ]
        });
        let tools = parse_tools_list_result(&result);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "read_file");
        assert_eq!(tools[0].description, "Read a file");
        assert_eq!(tools[1].name, "write_file");
        assert_eq!(tools[1].description, "");
    }

    #[test]
    fn parse_tools_list_result_missing_array_robust() {
        // No `tools` key → empty list, not panic.
        assert!(parse_tools_list_result(&json!({})).is_empty());
        // `tools` but wrong type → empty list.
        assert!(parse_tools_list_result(&json!({"tools": "wrong"})).is_empty());
    }

    #[test]
    fn parse_tools_list_result_skips_malformed_entries_robust() {
        // First entry missing `name`; should be silently dropped, not
        // poison the entire list.
        let result = json!({
            "tools": [
                { "description": "no name here" },
                { "name": "good_one", "inputSchema": {"type":"object"} }
            ]
        });
        let tools = parse_tools_list_result(&result);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "good_one");
    }

    #[test]
    fn parse_tools_call_result_text_normal() {
        let result = json!({
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "text", "text": "world"}
            ],
            "isError": false
        });
        let outcome = parse_tools_call_result(&result);
        assert_eq!(outcome.text, "hello\nworld");
        assert!(!outcome.is_error);
    }

    #[test]
    fn parse_tools_call_result_error_flag_normal() {
        let result = json!({
            "content": [{"type": "text", "text": "boom"}],
            "isError": true
        });
        let outcome = parse_tools_call_result(&result);
        assert!(outcome.is_error);
        assert_eq!(outcome.text, "boom");
    }

    #[test]
    fn parse_tools_call_result_drops_non_text_robust() {
        let result = json!({
            "content": [
                {"type": "image", "data": "base64..."},
                {"type": "text", "text": "kept"}
            ]
        });
        let outcome = parse_tools_call_result(&result);
        assert_eq!(outcome.text, "kept");
    }

    #[test]
    fn parse_tools_call_result_empty_content_robust() {
        let result = json!({});
        let outcome = parse_tools_call_result(&result);
        assert_eq!(outcome.text, "");
        assert!(!outcome.is_error);
    }

    #[test]
    fn advertise_tool_name_format_normal() {
        assert_eq!(
            advertise_tool_name("filesystem", "read_file"),
            "mcp__filesystem__read_file"
        );
    }

    #[test]
    fn split_advertised_roundtrips_normal() {
        let advertised = advertise_tool_name("git", "status");
        let (server, tool) = split_advertised(&advertised).expect("split");
        assert_eq!(server, "git");
        assert_eq!(tool, "status");
    }

    #[test]
    fn split_advertised_handles_double_underscore_in_tool_normal() {
        // Tool name itself contains `__` — splitter must take the FIRST
        // separator so the server name lands cleanly on the left.
        let advertised = "mcp__git__nested__op";
        let (server, tool) = split_advertised(advertised).expect("split");
        assert_eq!(server, "git");
        assert_eq!(tool, "nested__op");
    }

    #[test]
    fn split_advertised_rejects_non_mcp_robust() {
        // No `mcp__` prefix.
        assert!(split_advertised("read_file").is_none());
        assert!(split_advertised("Bash").is_none());
    }

    #[test]
    fn split_advertised_rejects_missing_separator_robust() {
        // Has prefix but no `__` after the server segment.
        assert!(split_advertised("mcp__filesystem").is_none());
        // Empty server / tool segment.
        assert!(split_advertised("mcp____tool").is_none());
        assert!(split_advertised("mcp__server__").is_none());
    }

    #[test]
    fn rpc_request_serde_roundtrip_normal() {
        let req = RpcRequest {
            jsonrpc: "2.0".into(),
            id: 1,
            method: "initialize".into(),
            params: Some(json!({"k": "v"})),
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: RpcRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn rpc_response_with_error_normal() {
        let s = r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"method not found"}}"#;
        let resp: RpcResponse = serde_json::from_str(s).unwrap();
        assert_eq!(resp.id, 3);
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "method not found");
    }
}
