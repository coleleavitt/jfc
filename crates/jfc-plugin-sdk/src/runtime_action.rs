use serde::{Deserialize, Serialize};

use crate::{PluginId, descriptor::DescriptorVisibility};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeActionKind {
    HostAction,
    SlashCommand,
    RefreshMetrics,
    OpenPanel,
    SendTeammateMessage,
    RefreshPromptContext,
    PluginSmoke,
    PluginDiagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeActionOpenPanelTarget {
    InfoSidebar,
    SessionsSidebar,
    ModelPicker,
    ThemePicker,
}

impl RuntimeActionOpenPanelTarget {
    pub fn parse(value: &str) -> Option<Self> {
        let _linkscope_parse =
            linkscope::phase("plugin_sdk.runtime_action.open_panel_target.parse");
        let parsed = match value {
            "info" | "info_sidebar" | "right_sidebar" => Some(Self::InfoSidebar),
            "sessions" | "sessions_sidebar" => Some(Self::SessionsSidebar),
            "model_picker" => Some(Self::ModelPicker),
            "theme_picker" => Some(Self::ThemePicker),
            _ => None,
        };
        linkscope::detail_event_fields(
            "plugin_sdk.runtime_action.open_panel_target.parse",
            [
                linkscope::TraceField::text("value", value.to_owned()),
                linkscope::TraceField::count("matched", u64::from(parsed.is_some())),
            ],
        );
        parsed
    }

    pub fn as_payload_str(self) -> &'static str {
        match self {
            Self::InfoSidebar => "info_sidebar",
            Self::SessionsSidebar => "sessions_sidebar",
            Self::ModelPicker => "model_picker",
            Self::ThemePicker => "theme_picker",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeActionDescriptor {
    pub plugin_id: PluginId,
    pub id: String,
    pub label: String,
    pub description: String,
    pub kind: RuntimeActionKind,
    pub priority: i32,
    pub visibility: DescriptorVisibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl RuntimeActionDescriptor {
    pub fn new(
        plugin_id: PluginId,
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
        kind: RuntimeActionKind,
    ) -> Self {
        let _linkscope_action = linkscope::phase("plugin_sdk.runtime_action.new");
        let id = id.into();
        let label = label.into();
        let description = description.into();
        linkscope::event_fields(
            "plugin_sdk.runtime_action.new",
            [
                linkscope::TraceField::text("plugin_id", plugin_id.as_str().to_owned()),
                linkscope::TraceField::text("id", id.clone()),
                linkscope::TraceField::text("kind", format!("{kind:?}")),
                linkscope::TraceField::bytes(
                    "label_bytes",
                    u64::try_from(label.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::bytes(
                    "description_bytes",
                    u64::try_from(description.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Self {
            plugin_id,
            id,
            label,
            description,
            kind,
            priority: 0,
            visibility: DescriptorVisibility::HostVisible,
            payload: None,
        }
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_visibility(mut self, visibility: DescriptorVisibility) -> Self {
        self.visibility = visibility;
        self
    }

    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        let _linkscope_payload = linkscope::phase("plugin_sdk.runtime_action.with_payload");
        linkscope::detail_event_fields(
            "plugin_sdk.runtime_action.with_payload",
            [
                linkscope::TraceField::text("id", self.id.clone()),
                linkscope::TraceField::text(
                    "payload_kind",
                    if payload.is_object() {
                        "object"
                    } else if payload.is_array() {
                        "array"
                    } else {
                        "scalar"
                    },
                ),
            ],
        );
        self.payload = Some(payload);
        self
    }

    pub fn with_host_action(mut self, action: impl Into<String>) -> Self {
        let _linkscope_payload = linkscope::phase("plugin_sdk.runtime_action.with_host_action");
        let action = action.into();
        linkscope::detail_event_fields(
            "plugin_sdk.runtime_action.with_host_action",
            [
                linkscope::TraceField::text("id", self.id.clone()),
                linkscope::TraceField::bytes(
                    "action_bytes",
                    u64::try_from(action.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        self.payload = Some(serde_json::json!({ "action": action }));
        self
    }

    pub fn with_slash_command(mut self, command: impl Into<String>) -> Self {
        let _linkscope_payload = linkscope::phase("plugin_sdk.runtime_action.with_slash_command");
        let command = command.into();
        linkscope::detail_event_fields(
            "plugin_sdk.runtime_action.with_slash_command",
            [
                linkscope::TraceField::text("id", self.id.clone()),
                linkscope::TraceField::bytes(
                    "command_bytes",
                    u64::try_from(command.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        self.payload = Some(serde_json::json!({ "command": command }));
        self
    }

    pub fn with_open_panel_target(mut self, target: RuntimeActionOpenPanelTarget) -> Self {
        self.payload = Some(serde_json::json!({ "panel": target.as_payload_str() }));
        self
    }

    pub fn with_plugin_smoke_target(mut self, plugin: impl Into<String>) -> Self {
        self.payload = Some(serde_json::json!({ "plugin": plugin.into() }));
        self
    }
}
