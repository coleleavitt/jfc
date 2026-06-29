use serde::{Deserialize, Serialize};

use super::SessionEntryId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelEntry {
    pub target_id: SessionEntryId,
    #[serde(default)]
    pub label: Option<String>,
}

impl LabelEntry {
    pub fn new(target_id: SessionEntryId) -> Self {
        Self {
            target_id,
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}
