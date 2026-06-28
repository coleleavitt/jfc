use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolUse {
    pub tool_use_id: String,
    pub kind: String,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

impl ToolUse {
    pub fn new(
        tool_use_id: impl Into<String>,
        kind: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            kind: kind.into(),
            input,
            thought_signature: None,
        }
    }

    pub fn with_thought_signature(mut self, thought_signature: impl Into<String>) -> Self {
        self.thought_signature = Some(thought_signature.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub status: String,
    #[serde(default)]
    pub output: serde_json::Value,
}

impl ToolResult {
    pub fn new(
        tool_use_id: impl Into<String>,
        status: impl Into<String>,
        output: serde_json::Value,
    ) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            status: status.into(),
            output,
        }
    }
}
