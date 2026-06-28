use serde::{Deserialize, Serialize};

use super::SessionEntryId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionBoundary {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_kept_entry_id: Option<SessionEntryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u64>,
}

impl CompactionBoundary {
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            first_kept_entry_id: None,
            tokens_before: None,
        }
    }
}
