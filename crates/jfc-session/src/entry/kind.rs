use serde::{Deserialize, Serialize};

use super::{
    BranchForkSummary, CompactionBoundary, ContextEvent, CustomPluginEntry, LabelEntry,
    MessageContentPart, MessageMetadata, ModelChange, SessionEntryValidationError, ThinkingChange,
    ToolResult, ToolUse,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntryKind {
    UserMessage {
        content: Vec<MessageContentPart>,
        #[serde(default)]
        metadata: MessageMetadata,
    },
    AssistantMessage {
        content: Vec<MessageContentPart>,
        #[serde(default)]
        metadata: MessageMetadata,
    },
    ToolUse(ToolUse),
    ToolResult(ToolResult),
    ModelChange(ModelChange),
    ThinkingChange(ThinkingChange),
    CompactionBoundary(CompactionBoundary),
    BranchForkSummary(BranchForkSummary),
    CustomPluginEntry(CustomPluginEntry),
    Label(LabelEntry),
    ContextEvent(ContextEvent),
}

impl SessionEntryKind {
    pub fn user_message(content: Vec<MessageContentPart>) -> Self {
        Self::UserMessage {
            content,
            metadata: MessageMetadata::default(),
        }
    }

    pub fn user_message_with_metadata(
        content: Vec<MessageContentPart>,
        metadata: MessageMetadata,
    ) -> Self {
        Self::UserMessage { content, metadata }
    }

    pub fn assistant_message(content: Vec<MessageContentPart>) -> Self {
        Self::AssistantMessage {
            content,
            metadata: MessageMetadata::default(),
        }
    }

    pub fn assistant_message_with_metadata(
        content: Vec<MessageContentPart>,
        metadata: MessageMetadata,
    ) -> Self {
        Self::AssistantMessage { content, metadata }
    }

    pub fn validate(&self) -> Result<(), SessionEntryValidationError> {
        match self {
            Self::CustomPluginEntry(entry) => entry.validate(),
            _ => Ok(()),
        }
    }
}
