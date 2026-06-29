//! MCP value shapes jfc cares about, plus adapters to/from the `rmcp` SDK.
//!
//! Wire-level JSON-RPC 2.0 framing, the `initialize` handshake, and
//! `tools/list` pagination all live inside [`rmcp`] now — see
//! [`super::transport`]. This module is the thin boundary that converts
//! `rmcp`'s protocol types into the lightweight shapes the registry
//! caches and the streaming layer advertises:
//!
//! - [`McpTool`] — a cached tool entry built from [`rmcp::model::Tool`].
//!   We keep our own struct so the
//!   registry's `ToolDef` mapping and `/mcp list` display don't depend on
//!   `rmcp` types leaking through the public API.
//! - [`ToolCallOutcome`] — the flattened result of a `tools/call`, built
//!   from [`rmcp::model::CallToolResult`].
//! - [`advertise_tool_name`] / [`split_advertised`] — the
//!   `mcp__<server>__<tool>` namespacing scheme, normalized to provider-safe
//!   ASCII while the registry keeps the original MCP names for dispatch.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One tool exposed by a connected MCP server. The model-facing `ToolDef`
/// path still uses name/description/input_schema, but the registry keeps
/// richer MCP metadata so app/resource surfaces can inspect it later.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_input_schema", rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(
        default,
        rename = "outputSchema",
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icons: Option<Value>,
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

impl Default for McpTool {
    fn default() -> Self {
        Self {
            name: String::new(),
            title: None,
            description: String::new(),
            input_schema: default_input_schema(),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }
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
            title: t.title,
            description: t.description.map(|d| d.into_owned()).unwrap_or_default(),
            // `input_schema` is an `Arc<Map<String, Value>>`; clone the
            // map out so we own a plain `Value::Object`.
            input_schema: normalize_input_schema(Value::Object((*t.input_schema).clone())),
            output_schema: t
                .output_schema
                .map(|schema| Value::Object((*schema).clone())),
            annotations: t.annotations.and_then(to_json_value),
            execution: t.execution.and_then(to_json_value),
            icons: t.icons.and_then(to_json_value),
            meta: t.meta.and_then(to_json_value),
        }
    }
}

fn to_json_value<T: Serialize>(value: T) -> Option<Value> {
    serde_json::to_value(value).ok()
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

const MAX_ADVERTISED_TOOL_NAME_LEN: usize = 64;

fn stable_hash_hex(input: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", hash as u32)
}

fn truncate_ascii(s: &str, max: usize) -> String {
    s.as_bytes()
        .iter()
        .take(max)
        .map(|b| char::from(*b))
        .collect()
}

/// Sanitize one advertised-name component to the provider-compatible tool-name
/// subset (`[A-Za-z0-9_-]`) while avoiding the `__` namespace separator inside
/// components. A hash suffix is added when the component changed so different
/// original MCP names do not collapse to the same advertised name.
pub fn sanitize_advertised_component(component: &str) -> String {
    let mut out = String::with_capacity(component.len().min(32));
    let mut changed = false;
    let mut last_underscore = false;
    for ch in component.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            ch
        } else {
            changed = true;
            '_'
        };
        if mapped == '_' {
            if last_underscore {
                changed = true;
                continue;
            }
            last_underscore = true;
        } else {
            last_underscore = false;
        }
        out.push(mapped);
    }
    let trimmed = out.trim_matches('_').to_owned();
    if trimmed.len() != out.len() {
        changed = true;
    }
    out = if trimmed.is_empty() {
        changed = true;
        "x".to_owned()
    } else {
        trimmed
    };
    if changed {
        out.push('_');
        out.push_str(&stable_hash_hex(component));
    }
    out
}

/// Construct the canonical provider-facing `mcp__<server>__<tool>` advertised
/// name. The registry maps this sanitized name back to the original MCP
/// `(server, tool)` pair before dispatch, so MCP servers can expose names with
/// dots, slashes, spaces, or other characters providers reject.
pub fn advertise_tool_name(server: &str, tool: &str) -> String {
    let server = sanitize_advertised_component(server);
    let tool = sanitize_advertised_component(tool);
    let full = format!("mcp__{server}__{tool}");
    if full.len() <= MAX_ADVERTISED_TOOL_NAME_LEN {
        return full;
    }

    let suffix = format!("_h{}", stable_hash_hex(&format!("{server}\0{tool}")));
    let fixed = "mcp__".len() + "__".len() + suffix.len();
    let budget = MAX_ADVERTISED_TOOL_NAME_LEN.saturating_sub(fixed).max(2);
    let server_budget = (budget / 3).max(1).min(server.len());
    let tool_budget = budget.saturating_sub(server_budget).max(1);
    let server = truncate_ascii(&server, server_budget);
    let tool = truncate_ascii(&tool, tool_budget.min(tool.len()));
    format!("mcp__{server}__{tool}{suffix}")
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
    fn mcp_tool_retains_rich_metadata_normal() {
        let tool = Tool::new("search", "Search", schema_obj())
            .with_raw_output_schema(schema_obj())
            .with_annotations(rmcp::model::ToolAnnotations::new().read_only(true));
        let mcp: McpTool = tool.into();
        assert!(mcp.output_schema.is_some());
        assert_eq!(
            mcp.annotations
                .as_ref()
                .and_then(|value| value.get("readOnlyHint")),
            Some(&json!(true))
        );
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
    fn advertise_tool_name_sanitizes_invalid_provider_names_regression() {
        let advertised = advertise_tool_name("git.server", "branch/list:all");
        assert!(advertised.starts_with("mcp__git_server_"));
        assert!(
            advertised
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "{advertised}"
        );
        assert!(advertised.len() <= 64, "{advertised}");
    }

    #[test]
    fn advertise_tool_name_caps_long_names_robust() {
        let advertised = advertise_tool_name(&"server".repeat(20), &"tool".repeat(30));
        assert!(advertised.len() <= 64, "{advertised}");
        assert!(split_advertised(&advertised).is_some());
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
