use serde::{Deserialize, de::Error as DeError};
use serde_json::Value;

use jfc_provider::ServerToolResultKind;

#[derive(Debug)]
pub enum ContentBlock {
    Text {
        #[allow(dead_code)]
        text: String,
    },
    Thinking {
        #[allow(dead_code)]
        thinking: String,
    },
    /// Server-redacted thinking block — opaque base64 blob, no deltas.
    /// Must be round-tripped verbatim in subsequent requests.
    RedactedThinking { data: String },
    ToolUse {
        id: String,
        name: String,
        #[allow(dead_code)]
        input: Value,
    },
    /// Anthropic server-side tool invocation (e.g. web_search, code_execution).
    /// These are executed server-side; jfc renders them but does not dispatch
    /// them locally. Shape mirrors `tool_use` but uses `server_tool_use` type.
    ServerToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Server-side tool result blocks (`web_search_tool_result`,
    /// `code_execution_tool_result`, `tool_search_tool_result`, ...).
    /// `tool_kind` preserves the original wire type so future Anthropic
    /// result blocks can round-trip without a parser release.
    ServerToolResult {
        tool_use_id: String,
        tool_kind: ServerToolResultKind,
        content: Value,
    },
    /// Forward-compatible catch-all for new content block types. Unknown
    /// blocks are ignored unless they carry `tool_use_id` + `content`, in
    /// which case they are promoted to `ServerToolResult` above.
    Unknown { kind: String, raw: Value },
}

impl<'de> Deserialize<'de> for ContentBlock {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| D::Error::missing_field("type"))?
            .to_owned();

        macro_rules! block {
            ($ty:ty) => {
                serde_json::from_value::<$ty>(value.clone()).map_err(D::Error::custom)?
            };
        }

        match kind.as_str() {
            "text" => {
                #[derive(Deserialize)]
                struct Block {
                    #[serde(default)]
                    text: String,
                }
                let block = block!(Block);
                Ok(Self::Text { text: block.text })
            }
            "thinking" => {
                #[derive(Deserialize)]
                struct Block {
                    #[serde(default)]
                    thinking: String,
                }
                let block = block!(Block);
                Ok(Self::Thinking {
                    thinking: block.thinking,
                })
            }
            "redacted_thinking" => {
                #[derive(Deserialize)]
                struct Block {
                    data: String,
                }
                let block = block!(Block);
                Ok(Self::RedactedThinking { data: block.data })
            }
            "tool_use" => {
                #[derive(Deserialize)]
                struct Block {
                    id: String,
                    name: String,
                    #[serde(default)]
                    input: Value,
                }
                let block = block!(Block);
                Ok(Self::ToolUse {
                    id: block.id,
                    name: block.name,
                    input: block.input,
                })
            }
            "server_tool_use" => {
                #[derive(Deserialize)]
                struct Block {
                    id: String,
                    name: String,
                    #[serde(default)]
                    input: Value,
                }
                let block = block!(Block);
                Ok(Self::ServerToolUse {
                    id: block.id,
                    name: block.name,
                    input: block.input,
                })
            }
            "web_search_tool_result"
            | "code_execution_tool_result"
            | "web_fetch_tool_result"
            | "advisor_tool_result"
            | "bash_code_execution_tool_result"
            | "text_editor_code_execution_tool_result"
            | "tool_search_tool_result" => {
                #[derive(Deserialize)]
                struct Block {
                    tool_use_id: String,
                    #[serde(default)]
                    content: Value,
                }
                let block = block!(Block);
                Ok(Self::ServerToolResult {
                    tool_use_id: block.tool_use_id,
                    tool_kind: ServerToolResultKind::from_wire_type(&kind),
                    content: block.content,
                })
            }
            _ => {
                if let Some(tool_use_id) = value.get("tool_use_id").and_then(Value::as_str) {
                    return Ok(Self::ServerToolResult {
                        tool_use_id: tool_use_id.to_owned(),
                        tool_kind: ServerToolResultKind::from_wire_type(&kind),
                        content: value.get("content").cloned().unwrap_or(Value::Null),
                    });
                }
                Ok(Self::Unknown { kind, raw: value })
            }
        }
    }
}
