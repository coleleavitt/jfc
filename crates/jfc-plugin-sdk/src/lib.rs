//! Stable plugin contract crate for JFC.
//!
//! This crate intentionally contains DTOs, newtypes, descriptors, and typed
//! hook/capability names only. Stable API: manifest identity, source metadata,
//! capabilities, descriptors, compatibility reports, and process bridge frames.
//! Experimental API: UI-agnostic extension slots, which name host-owned regions
//! without exposing ratatui/crossterm widgets or any concrete frontend type.

pub mod agent_launch;
pub mod bridge;
pub mod capability;
pub mod compat;
pub mod descriptor;
pub mod error;
pub mod hook;
pub mod manifest;
pub mod metric;
pub mod provider_bridge;
pub mod runtime_action;
pub mod runtime_action_payload;
pub mod runtime_extension;
pub mod service;
pub mod source;
pub mod teammate;
pub mod ui_panel;
pub mod ui_widget;

pub use agent_launch::{
    AgentLaunchDescriptor, AgentLaunchExecutorDescriptor, AgentLaunchExecutorKind,
    BridgeAgentLaunchRequest, BridgeAgentLaunchResult, BridgeTeammateEvent,
};
pub use bridge::{
    BridgeEnvelope, BridgeErrorDto, BridgeRequest, BridgeResponse, ProcessBridgeCommand,
};
pub use capability::{ExtensionSlot, PluginCapability, UiSlotActionDescriptor, UiSlotDescriptor};
pub use compat::{CompatibilityErrorDto, CompatibilityReport, CompatibilityStatus};
pub use descriptor::{
    AuthDescriptor, AuthMethodDescriptor, CommandDescriptor, DescriptorVisibility,
    ProviderDescriptor, ProviderExecutorDescriptor, ProviderExecutorKind, ProviderModelDescriptor,
    ResourceDescriptor, ResourceKind, ToolApprovalPolicy, ToolDescriptor, ToolExecutorDescriptor,
    ToolExecutorKind,
};
pub use error::{IdentifierError, PluginSdkError};
pub use hook::{HookDescriptor, HookName};
pub use manifest::{PluginId, PluginManifest, PluginVersion};
pub use metric::{MetricDescriptor, MetricSurface, MetricUnit};
pub use provider_bridge::{
    BridgeFallbackReason, BridgeProviderContent, BridgeProviderMessage, BridgeProviderRole,
    BridgeProviderStreamEvent, BridgeProviderStreamOptions, BridgeProviderToolDef,
    BridgeStopReason,
};
pub use runtime_action::{
    RuntimeActionDescriptor, RuntimeActionKind, RuntimeActionOpenPanelTarget,
};
pub use runtime_action_payload::{RuntimeActionOpenPanelPayload, RuntimeActionPayloadError};
pub use runtime_extension::{
    BridgePromptContextRefreshRequest, BridgePromptContextRefreshResult,
    RuntimeExtensionDescriptor, RuntimeExtensionExecutorDescriptor, RuntimeExtensionExecutorKind,
    RuntimeExtensionRefreshDescriptor, RuntimeExtensionRefreshKind, RuntimeExtensionTarget,
};
pub use service::{ServiceDescriptor, ServiceDescriptorKind, ServiceDescriptorStatus};
pub use source::{PluginScope, PluginSource};
pub use teammate::{
    BridgeMailboxMessage, BridgeMailboxPollRequest, BridgeMailboxSendRequest, BridgeTeammateReady,
};
pub use ui_panel::{
    BridgeUiPanelRefreshRequest, BridgeUiPanelRefreshResult, UiPanelDescriptor,
    UiPanelRefreshDescriptor, UiPanelRefreshKind,
};
pub use ui_widget::{
    BridgeUiWidgetRefreshRequest, BridgeUiWidgetRefreshResult, UiMutationScope, UiWidgetDescriptor,
    UiWidgetKind, UiWidgetRefreshDescriptor, UiWidgetRefreshKind,
};
