//! MCP value shapes jfc cares about, plus adapters to/from the `rmcp` SDK.
//!
//! Wire-level JSON-RPC 2.0 framing, the `initialize` handshake, and
//! `tools/list` pagination all live inside [`rmcp`] now — see
//! [`super::transport`]. This module is the thin boundary that converts
//! `rmcp`'s protocol types into the lightweight shapes the registry
//! caches and the streaming layer advertises:
//!
//! - [`McpTool`] — a cached tool entry (`{name, description, inputSchema}`)
//!   built from [`rmcp::model::Tool`]. We keep our own struct so the
//!   registry's `ToolDef` mapping and `/mcp list` display don't depend on
//!   `rmcp` types leaking through the public API.
//! - [`ToolCallOutcome`] — the flattened result of a `tools/call`, built
//!   from [`rmcp::model::CallToolResult`].
//! - [`advertise_tool_name`] / [`split_advertised`] — the
//!   `mcp__<server>__<tool>` namespacing scheme, unchanged.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One tool exposed by a connected MCP server. Mirrors the fields jfc
/// advertises to the model; richer `rmcp` metadata (annotations, output
/// schema, icons) is dropped at this boundary.
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

fn normalize_input_schema(mut schema: Value) -> Value {
    if let Value::Object(object) = &mut schema {
        // rmcp 8f558d8 strips these from generated tools/list schemas.
        // Do the same at the client boundary so non-rmcp MCP servers don't
        // leak redundant top-level metadata into provider tool schemas.
        object.remove("title");
        object.remove("description");
    }
    schema
}

impl From<rmcp::model::Tool> for McpTool {
    fn from(t: rmcp::model::Tool) -> Self {
        Self {
            name: t.name.into_owned(),
            description: t.description.map(|d| d.into_owned()).unwrap_or_default(),
            // `input_schema` is an `Arc<Map<String, Value>>`; clone the
            // map out so we own a plain `Value::Object`.
            input_schema: normalize_input_schema(Value::Object((*t.input_schema).clone())),
        }
    }
}

/// Flattened result of a `tools/call`. `is_error` mirrors the MCP
/// `isError` field — true means the tool ran but reported a failure (the
/// JSON-RPC layer itself succeeded). Transport-level errors come back as
/// [`super::transport::RequestError`] and are handled by the dispatcher.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallOutcome {
    pub text: String,
    pub is_error: bool,
}

impl From<rmcp::model::CallToolResult> for ToolCallOutcome {
    /// Concatenate every text content block. Non-text content (images,
    /// embedded resources) is currently dropped — TODO: surface images
    /// as attachments. If the server returned only `structuredContent`
    /// and no text blocks, fall back to its JSON serialization so the
    /// model still sees something.
    fn from(result: rmcp::model::CallToolResult) -> Self {
        let is_error = result.is_error.unwrap_or(false);
        let mut text = String::new();
        for block in &result.content {
            if let Some(t) = block.as_text() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&t.text);
            }
        }
        if text.is_empty()
            && let Some(structured) = result.structured_content.as_ref()
        {
            text = structured.to_string();
        }
        Self { text, is_error }
    }
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
    use rmcp::model::{CallToolResult, Content, Tool};
    use std::sync::Arc;

    fn schema_obj() -> Arc<serde_json::Map<String, Value>> {
        let mut schema = serde_json::Map::new();
        schema.insert("type".into(), json!("object"));
        Arc::new(schema)
    }

    #[test]
    fn mcp_tool_from_rmcp_tool_normal() {
        let tool = Tool::new("read_file", "Read a file", schema_obj());
        let mcp: McpTool = tool.into();
        assert_eq!(mcp.name, "read_file");
        assert_eq!(mcp.description, "Read a file");
        assert_eq!(mcp.input_schema["type"], "object");
    }

    #[test]
    fn mcp_tool_from_rmcp_tool_missing_description_robust() {
        let tool = Tool::new_with_raw("noop", None, Arc::new(serde_json::Map::new()));
        let mcp: McpTool = tool.into();
        assert_eq!(mcp.description, "");
    }

    #[test]
    fn mcp_tool_strips_redundant_top_level_input_schema_metadata_normal() {
        let mut schema = serde_json::Map::new();
        schema.insert(
            "$schema".into(),
            json!("https://json-schema.org/draft/2020-12/schema"),
        );
        schema.insert("title".into(), json!("AddRequest"));
        schema.insert(
            "description".into(),
            json!("Parameters for adding two numbers."),
        );
        schema.insert("type".into(), json!("object"));
        schema.insert(
            "properties".into(),
            json!({
                "a": {
                    "description": "The left-hand number.",
                    "type": "number"
                }
            }),
        );

        let tool = Tool::new("add", "Add two numbers.", Arc::new(schema));
        let mcp: McpTool = tool.into();

        assert!(mcp.input_schema.get("title").is_none());
        assert!(mcp.input_schema.get("description").is_none());
        assert_eq!(
            mcp.input_schema["properties"]["a"]["description"],
            "The left-hand number."
        );
    }

    #[test]
    fn tool_call_outcome_concatenates_text_normal() {
        let result = CallToolResult::success(vec![Content::text("hello"), Content::text("world")]);
        let outcome: ToolCallOutcome = result.into();
        assert_eq!(outcome.text, "hello\nworld");
        assert!(!outcome.is_error);
    }

    #[test]
    fn tool_call_outcome_error_flag_normal() {
        let result = CallToolResult::error(vec![Content::text("boom")]);
        let outcome: ToolCallOutcome = result.into();
        assert!(outcome.is_error);
        assert_eq!(outcome.text, "boom");
    }

    #[test]
    fn tool_call_outcome_reads_structured_text_normal() {
        // `structured` mirrors the JSON into a text content block, so the
        // flattened outcome surfaces it to the model.
        let result = CallToolResult::structured(json!({"ok": true}));
        let outcome: ToolCallOutcome = result.into();
        assert!(outcome.text.contains("\"ok\""));
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
        assert!(split_advertised("read_file").is_none());
        assert!(split_advertised("Bash").is_none());
    }

    #[test]
    fn split_advertised_rejects_missing_separator_robust() {
        assert!(split_advertised("mcp__filesystem").is_none());
        assert!(split_advertised("mcp____tool").is_none());
        assert!(split_advertised("mcp__server__").is_none());
    }
}
