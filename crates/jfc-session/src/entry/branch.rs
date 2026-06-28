use serde::{Deserialize, Serialize};

use super::SessionEntryId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchForkSummary {
    pub from_id: SessionEntryId,
    pub summary: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

impl BranchForkSummary {
    pub fn new(
        from_id: SessionEntryId,
        summary: impl Into<String>,
        details: serde_json::Value,
    ) -> Self {
        Self {
            from_id,
            summary: summary.into(),
            details,
        }
    }
}
