use serde::{Deserialize, Serialize};

use super::codec::deserialize_non_empty_string;
use super::validation::{self, SessionEntryValidationError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomPluginEntry {
    #[serde(deserialize_with = "deserialize_non_empty_string")]
    pub plugin_id: String,
    #[serde(deserialize_with = "deserialize_non_empty_string")]
    pub custom_type: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl CustomPluginEntry {
    pub fn new(
        plugin_id: impl Into<String>,
        custom_type: impl Into<String>,
        data: serde_json::Value,
    ) -> Result<Self, SessionEntryValidationError> {
        let entry = Self {
            plugin_id: plugin_id.into(),
            custom_type: custom_type.into(),
            data,
        };
        entry.validate()?;
        Ok(entry)
    }

    pub fn validate(&self) -> Result<(), SessionEntryValidationError> {
        validation::validate_non_empty(
            &self.plugin_id,
            SessionEntryValidationError::EmptyPluginId,
        )?;
        validation::validate_non_empty(
            &self.custom_type,
            SessionEntryValidationError::EmptyCustomType,
        )
    }
}
