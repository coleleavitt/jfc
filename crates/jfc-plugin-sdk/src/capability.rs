use serde::{Deserialize, Serialize};

use crate::{
    HookName, MetricSurface, PluginId, RuntimeExtensionTarget,
    agent_launch::AgentLaunchExecutorKind, descriptor::DescriptorVisibility,
    runtime_action::RuntimeActionKind, ui_widget::UiMutationScope,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginCapability {
    Tools,
    Providers,
    Resources,
    Commands,
    Auth,
    Bridge,
    Governance,
    Audit,
    Background,
    Remote,
    Design,
    Voice,
    FrontendSupport,
    PluginManagement,
    AgentLaunches {
        executors: Vec<AgentLaunchExecutorKind>,
    },
    RuntimeExtensions {
        targets: Vec<RuntimeExtensionTarget>,
    },
    RuntimeActions {
        actions: Vec<RuntimeActionKind>,
    },
    UiPanels {
        scopes: Vec<UiMutationScope>,
    },
    Metrics {
        surfaces: Vec<MetricSurface>,
    },
    UiWidgets {
        scopes: Vec<UiMutationScope>,
    },
    Hooks {
        hooks: Vec<HookName>,
    },
    UiSlots {
        slots: Vec<ExtensionSlot>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionSlot {
    StatusLine,
    CommandPalette,
    MessageRenderer,
    Notification,
    PromptContext,
    TranscriptAnnotation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UiSlotDescriptor {
    pub plugin_id: PluginId,
    pub slot: ExtensionSlot,
    pub id: String,
    pub label: String,
    pub priority: i32,
    pub visibility: DescriptorVisibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<UiSlotActionDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiSlotActionDescriptor {
    HostAction { action: String },
    SlashCommand { command: String },
}

impl UiSlotDescriptor {
    pub fn new(
        plugin_id: PluginId,
        slot: ExtensionSlot,
        id: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        Self {
            plugin_id,
            slot,
            id: id.into(),
            label: label.into(),
            priority: 0,
            visibility: DescriptorVisibility::HostVisible,
            action: None,
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

    pub fn with_action(mut self, action: UiSlotActionDescriptor) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_host_action(self, action: impl Into<String>) -> Self {
        self.with_action(UiSlotActionDescriptor::HostAction {
            action: action.into(),
        })
    }

    pub fn with_slash_command(self, command: impl Into<String>) -> Self {
        self.with_action(UiSlotActionDescriptor::SlashCommand {
            command: command.into(),
        })
    }
}
