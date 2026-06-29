use jfc_plugin_sdk::PluginId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDescriptorIssueKind {
    MissingRuntimeAction,
    MissingUiPanel,
    MissingUiWidget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDescriptorIssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDescriptorIssueActionability {
    AddRuntimeAction,
    AddUiPanel,
    AddUiWidget,
    FixReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginDescriptorRepairAction {
    AddRuntimeAction {
        plugin_id: PluginId,
        action_id: String,
    },
    AddUiPanel {
        plugin_id: PluginId,
        panel_id: String,
    },
    AddUiWidget {
        plugin_id: PluginId,
        widget_id: String,
    },
    FixReference {
        plugin_id: PluginId,
        descriptor_kind: PluginDescriptorKind,
        descriptor_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDescriptorKind {
    RuntimeAction,
    UiPanel,
    UiSlot,
    UiWidget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDescriptorTargetKind {
    RuntimeAction,
    UiPanel,
    UiWidget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginDescriptorIssue {
    pub kind: PluginDescriptorIssueKind,
    pub severity: PluginDescriptorIssueSeverity,
    pub actionability: PluginDescriptorIssueActionability,
    pub plugin_id: PluginId,
    pub descriptor_kind: PluginDescriptorKind,
    pub descriptor_id: String,
    pub target_plugin_id: PluginId,
    pub target_kind: PluginDescriptorTargetKind,
    pub target_id: String,
    pub message: String,
    pub repair_action: PluginDescriptorRepairAction,
    pub repair_hint: String,
}
