use base64::Engine as _;
use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(super) struct ProviderHistoryArchive {
    pub(super) schema_version: u32,
    pub(super) id: String,
    pub(super) created_at: String,
    pub(super) pre_tokens: u64,
    pub(super) summary: String,
    pub(super) messages: Vec<ArchivedProviderMessage>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ArchivedProviderMessage {
    pub(super) role: ArchivedProviderRole,
    pub(super) content: Vec<ArchivedProviderContent>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ArchivedProviderRole {
    User,
    Assistant,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ArchivedProviderContent {
    Text {
        text: String,
    },
    Thinking {
        text: String,
        signature: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        thought_signature: Option<String>,
    },
    ServerToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ServerToolResult {
        tool_use_id: String,
        tool_kind: String,
        content: serde_json::Value,
    },
    Attachment {
        id: u32,
        mime_type: String,
        byte_len: usize,
        bytes_base64: String,
    },
    RedactedThinking {
        byte_len: usize,
        data_base64: String,
    },
}

impl From<&ProviderMessage> for ArchivedProviderMessage {
    fn from(message: &ProviderMessage) -> Self {
        Self {
            role: match message.role {
                ProviderRole::User => ArchivedProviderRole::User,
                ProviderRole::Assistant => ArchivedProviderRole::Assistant,
            },
            content: message
                .content
                .iter()
                .map(ArchivedProviderContent::from)
                .collect(),
        }
    }
}

impl From<&ProviderContent> for ArchivedProviderContent {
    fn from(content: &ProviderContent) -> Self {
        match content {
            ProviderContent::Text(text) => Self::Text { text: text.clone() },
            ProviderContent::Thinking { text, signature } => Self::Thinking {
                text: text.clone(),
                signature: signature.clone(),
            },
            ProviderContent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Self::ToolResult {
                tool_use_id: tool_use_id.clone(),
                content: content.clone(),
                is_error: *is_error,
            },
            ProviderContent::ToolUse {
                id,
                name,
                input,
                thought_signature,
            } => Self::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
                thought_signature: thought_signature.clone(),
            },
            ProviderContent::ServerToolUse { id, name, input } => Self::ServerToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            },
            ProviderContent::ServerToolResult {
                tool_use_id,
                tool_kind,
                content,
            } => Self::ServerToolResult {
                tool_use_id: tool_use_id.clone(),
                tool_kind: tool_kind.wire_type().to_owned(),
                content: content.clone(),
            },
            ProviderContent::Attachment(attachment) => Self::Attachment {
                id: attachment.id,
                mime_type: attachment.kind.mime_type().to_owned(),
                byte_len: attachment.bytes.len(),
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(&attachment.bytes),
            },
            ProviderContent::RedactedThinking { data } => Self::RedactedThinking {
                byte_len: data.len(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(data.as_bytes()),
            },
        }
    }
}
