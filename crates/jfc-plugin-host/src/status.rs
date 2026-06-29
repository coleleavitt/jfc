use jfc_plugin_sdk::{HookDescriptor, PluginId, PluginManifest, PluginSource};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginStatusKind {
    Registered,
    Active,
    Disabled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginErrorPhase {
    Activation,
    Hook,
    Finalizer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginErrorReport {
    pub plugin_id: PluginId,
    pub phase: PluginErrorPhase,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginStatusEntry {
    pub plugin_id: PluginId,
    pub manifest: PluginManifest,
    pub source: PluginSource,
    pub status: PluginStatusKind,
    pub activation_order: i32,
    pub hooks: Vec<HookDescriptor>,
    pub errors: Vec<PluginErrorReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginHostSnapshot {
    pub plugins: Vec<PluginStatusEntry>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginHealthSummary {
    pub total: usize,
    pub active: usize,
    pub registered: usize,
    pub disabled: usize,
    pub failed: usize,
    pub error_count: usize,
}

impl PluginHostSnapshot {
    pub fn health_summary(&self) -> PluginHealthSummary {
        let mut summary = PluginHealthSummary::default();
        summary.total = self.plugins.len();
        for plugin in &self.plugins {
            match plugin.status {
                PluginStatusKind::Registered => summary.registered += 1,
                PluginStatusKind::Active => summary.active += 1,
                PluginStatusKind::Disabled => summary.disabled += 1,
                PluginStatusKind::Failed => summary.failed += 1,
            }
            summary.error_count += plugin.errors.len();
        }
        summary
    }
}
