use serde::{Deserialize, Serialize};

use crate::PluginId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    Compatible,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CompatibilityReport {
    pub plugin_id: PluginId,
    pub status: CompatibilityStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<CompatibilityErrorDto>,
}

impl CompatibilityReport {
    pub fn compatible(plugin_id: PluginId) -> Self {
        Self {
            plugin_id,
            status: CompatibilityStatus::Compatible,
            errors: Vec::new(),
        }
    }

    pub fn incompatible(plugin_id: PluginId, error: CompatibilityErrorDto) -> Self {
        Self {
            plugin_id,
            status: CompatibilityStatus::Incompatible,
            errors: vec![error],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CompatibilityErrorDto {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
}

impl CompatibilityErrorDto {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            plugin_id: None,
            expected: None,
            found: None,
        }
    }

    pub fn unsupported_manifest_schema(
        plugin_id: PluginId,
        found_schema_version: u16,
        supported_schema_version: u16,
    ) -> Self {
        Self::new(
            "unsupported_manifest_schema",
            "plugin manifest schema is newer than this host supports",
        )
        .with_plugin_id(plugin_id)
        .with_expected(format!("<= {supported_schema_version}"))
        .with_found(found_schema_version.to_string())
    }

    pub fn with_plugin_id(mut self, plugin_id: PluginId) -> Self {
        self.plugin_id = Some(plugin_id.into_inner());
        self
    }

    pub fn with_expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn with_found(mut self, found: impl Into<String>) -> Self {
        self.found = Some(found.into());
        self
    }
}
