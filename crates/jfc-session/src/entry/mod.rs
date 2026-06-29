mod branch;
mod codec;
mod compaction;
mod content;
mod context;
mod id;
mod kind;
mod label;
mod message;
mod model;
mod plugin;
mod tool;
mod validation;

use serde::{Deserialize, Serialize};

pub use branch::BranchForkSummary;
pub use compaction::CompactionBoundary;
pub use content::MessageContentPart;
pub use context::ContextEvent;
pub use id::SessionEntryId;
pub use kind::SessionEntryKind;
pub use label::LabelEntry;
pub use message::MessageMetadata;
pub use model::{ModelChange, ThinkingChange};
pub use plugin::CustomPluginEntry;
pub use tool::{ToolResult, ToolUse};
pub use validation::SessionEntryValidationError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: SessionEntryId,
    #[serde(default)]
    pub parent_id: Option<SessionEntryId>,
    pub timestamp: String,
    #[serde(flatten)]
    pub kind: SessionEntryKind,
}

impl SessionEntry {
    pub fn new(id: SessionEntryId, timestamp: impl Into<String>, kind: SessionEntryKind) -> Self {
        Self {
            id,
            parent_id: None,
            timestamp: timestamp.into(),
            kind,
        }
    }

    pub fn with_parent(mut self, parent_id: SessionEntryId) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    pub fn validate(&self) -> Result<(), SessionEntryValidationError> {
        self.id.validate()?;
        if let Some(parent_id) = &self.parent_id {
            parent_id.validate_as_parent()?;
        }
        validation::validate_non_empty(
            &self.timestamp,
            SessionEntryValidationError::EmptyTimestamp,
        )?;
        self.kind.validate()
    }
}
