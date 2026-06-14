//! `POST /v1/messages` — chat completions, streaming or non-streaming.
//!
//! Mirrors the Go SDK's `MessageService.New` shape. For streaming, callers
//! consume the returned `EventStream` and decode `MessageStreamEvent` values
//! per chunk. For non-streaming, callers await `MessageResponse`.

use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize)]
pub struct MessageRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub stop_sequences: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tools: Vec<ToolDef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_management: Option<ContextManagementConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    Image {
        source: ImageSource,
    },
    Document {
        source: DocumentSource,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub kind: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSource {
    #[serde(rename = "type")]
    pub kind: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// v132 server-side hosted tools — Anthropic-managed sandboxes the API
/// invokes on behalf of the model. Reference these by `name` in
/// `MessageRequest.tools` instead of a custom `ToolDef`. The model gets
/// a tool whose execution happens server-side; the response includes
/// the tool result inline.
pub mod hosted_tools {
    /// Bash code execution sandbox. Versioned by Anthropic; bump as
    /// the dated revision in the SDK constants moves forward.
    pub const BASH_CODE_EXECUTION: &str = "bash_20250124";
    /// Text-editor-aware code execution.
    pub const TEXT_EDITOR_CODE_EXECUTION: &str = "text_editor_20250124";
    /// Computer use (screenshot / click / type).
    pub const COMPUTER_USE: &str = "computer_20251124";
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    Auto,
    Any,
    Tool { name: String },
    None,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingConfig {
    Adaptive,
    Enabled { budget_tokens: u32 },
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextManagementConfig {
    pub edits: Vec<ContextEdit>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ContextEdit {
    #[serde(rename = "compact_20260112")]
    Compact20260112,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageResponse {
    pub id: String,
    pub model: String,
    pub role: Role,
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CountTokensResponse {
    pub input_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

pub struct MessageService {
    client: Client,
}

impl MessageService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Non-streaming: returns the full response body once the model finishes.
    pub async fn create(&self, mut req: MessageRequest) -> Result<MessageResponse> {
        req.stream = Some(false);
        self.create_with_betas(req, &[]).await
    }

    /// Non-streaming with explicit `anthropic-beta` tokens. This is the SDK
    /// escape hatch for beta-gated helpers such as structured outputs.
    pub async fn create_with_betas(
        &self,
        mut req: MessageRequest,
        betas: &[&str],
    ) -> Result<MessageResponse> {
        req.stream = Some(false);
        let beta_header = join_betas(betas);
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, "/v1/messages", beta_header.as_deref())
                    .json(&req)
            })
            .await?;
        Ok(resp.json::<MessageResponse>().await?)
    }

    /// Count tokens for a messages request without generating a response.
    pub async fn count_tokens(&self, mut req: MessageRequest) -> Result<CountTokensResponse> {
        req.stream = None;
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, "/v1/messages/count_tokens", None)
                    .json(&req)
            })
            .await?;
        Ok(resp.json::<CountTokensResponse>().await?)
    }

    /// Structured-output helper. The schema is passed through directly so
    /// callers can keep using their existing JSON Schema builder.
    pub async fn create_structured(
        &self,
        mut req: MessageRequest,
        name: &str,
        schema: serde_json::Value,
    ) -> Result<MessageResponse> {
        req.output_config = Some(json!({
            "format": {
                "type": "json_schema",
                "name": name,
                "schema": schema,
            }
        }));
        self.create_with_betas(req, &[crate::beta::STRUCTURED_OUTPUTS])
            .await
    }

    /// Server-side compaction helper. The response content must be preserved
    /// by callers because compaction blocks are part of the conversation state.
    pub async fn create_with_server_compaction(
        &self,
        mut req: MessageRequest,
    ) -> Result<MessageResponse> {
        req.context_management = Some(ContextManagementConfig {
            edits: vec![ContextEdit::Compact20260112],
        });
        self.create_with_betas(req, &[crate::beta::CONTEXT_MANAGEMENT])
            .await
    }
}

fn join_betas(betas: &[&str]) -> Option<String> {
    let tokens: Vec<_> = betas
        .iter()
        .map(|beta| beta.trim())
        .filter(|beta| !beta.is_empty())
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(","))
    }
}
