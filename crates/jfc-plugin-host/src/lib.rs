mod builtin_agent_workflow;
mod builtin_governance;
mod builtin_knowledge;
mod builtin_mcp;
mod builtin_metrics;
mod builtin_palette;
mod builtin_plugin_management;
mod builtin_prompt_context;
mod builtin_ux;
mod descriptor_issue_types;
mod descriptor_issues;
mod descriptors;
mod diagnostics;
mod discovery;
mod error;
mod hook;
mod host;
mod lifecycle;
mod manifest;
mod manifest_agent_launch;
mod manifest_metric;
mod manifest_provider;
mod manifest_runtime_action;
mod manifest_runtime_extension;
mod manifest_tool;
mod manifest_ui_panel;
mod manifest_ui_slot;
mod manifest_ui_widget;
mod process_bridge;
mod registration;
mod registration_activation;
mod resource_registration;
mod runtime;
mod state_cache;
mod status;
mod ui_widget_refresh;

pub use builtin_agent_workflow::{
    BUILTIN_AGENT_LAUNCH_HANDLER, BUILTIN_AGENT_LAUNCH_ID, BUILTIN_AGENT_RESOURCE_PATH,
    BUILTIN_AGENTS_PLUGIN_ID, BUILTIN_BACKGROUND_AGENT_LAUNCH_HANDLER,
    BUILTIN_BACKGROUND_AGENT_LAUNCH_ID, BUILTIN_SKILL_RESOURCE_PATH,
    BUILTIN_WORKFLOW_RESOURCE_PATH, BUILTIN_WORKFLOWS_PLUGIN_ID,
    builtin_agent_workflow_plugin_host, register_builtin_agent_workflow_plugins,
};
pub use builtin_governance::{builtin_governance_plugin_host, register_builtin_governance_plugins};
pub use builtin_knowledge::{
    BuiltinKnowledgeRegistrationReport, builtin_knowledge_plugin_host,
    register_builtin_knowledge_plugins,
};
pub use builtin_mcp::{
    BUILTIN_MCP_PLUGIN_ID, BUILTIN_TOOL_SERVICES_PLUGIN_ID, builtin_mcp_plugin,
    builtin_service_host, builtin_tool_services_plugin,
};
pub use builtin_metrics::{
    BUILTIN_CACHE_DIGEST_METRIC_ID, BUILTIN_CACHE_HIT_METRIC_ID, BUILTIN_OBSERVABILITY_PLUGIN_ID,
    BUILTIN_RSI_PROMPT_SECTIONS_METRIC_ID, BUILTIN_RSI_TOOL_VISIBILITY_METRIC_ID,
    builtin_observability_plugin, builtin_observability_plugin_host,
};
pub use builtin_plugin_management::{
    BUILTIN_PLUGIN_MANAGEMENT_PLUGIN_ID, builtin_plugin_management_plugin,
    builtin_plugin_management_plugin_host, plugin_management_plugin_host,
    register_builtin_plugin_management_plugin, reload_plugin_management_plugin_host,
};
pub use builtin_prompt_context::{
    BUILTIN_BACKGROUND_REMINDERS_PROMPT_CONTEXT_ID, BUILTIN_BACKGROUND_REMINDERS_PROMPT_HANDLER,
    BUILTIN_BRIEF_MODE_PROMPT_CONTEXT_ID, BUILTIN_BRIEF_MODE_PROMPT_HANDLER,
    BUILTIN_DOCUMENT_FORMATS_PROMPT_CONTEXT_ID, BUILTIN_DOCUMENT_FORMATS_PROMPT_HANDLER,
    BUILTIN_FEATURE_GATES_PROMPT_CONTEXT_ID, BUILTIN_FEATURE_GATES_PROMPT_HANDLER,
    BUILTIN_HARRIER_PROMPT_CONTEXT_ID, BUILTIN_HARRIER_PROMPT_HANDLER,
    BUILTIN_INTERACTION_MODE_PROMPT_CONTEXT_ID, BUILTIN_INTERACTION_MODE_PROMPT_HANDLER,
    BUILTIN_LOCAL_ADVISOR_PROMPT_CONTEXT_ID, BUILTIN_LOCAL_ADVISOR_PROMPT_HANDLER,
    BUILTIN_MARSH_PROMPT_CONTEXT_ID, BUILTIN_MARSH_PROMPT_HANDLER,
    BUILTIN_OUTPUT_STYLE_PROMPT_CONTEXT_ID, BUILTIN_OUTPUT_STYLE_PROMPT_HANDLER,
    BUILTIN_PEWTER_OWL_PROMPT_CONTEXT_ID, BUILTIN_PEWTER_OWL_PROMPT_HANDLER,
    BUILTIN_PREVIOUS_HANDOFF_PROMPT_CONTEXT_ID, BUILTIN_PREVIOUS_HANDOFF_PROMPT_HANDLER,
    BUILTIN_PROMPT_CONTEXT_PLUGIN_ID, BUILTIN_SERVER_ADVISOR_PROMPT_CONTEXT_ID,
    BUILTIN_SERVER_ADVISOR_PROMPT_HANDLER, BUILTIN_TOTAL_TOKENS_PROMPT_CONTEXT_ID,
    BUILTIN_TOTAL_TOKENS_PROMPT_HANDLER, builtin_prompt_context_plugin,
    builtin_prompt_context_plugin_host,
};
pub use builtin_ux::{
    BUILTIN_GOAL_STATUS_SLOT_ID, BUILTIN_MESSAGE_RENDERER_SLOT_ID, BUILTIN_PLUGIN_HEALTH_SLOT_ID,
    BuiltinUxRegistrationReport, builtin_status_line_plugin_host, builtin_ux_plugin_host,
    register_builtin_ux_plugins,
};
pub use descriptor_issue_types::{
    PluginDescriptorIssue, PluginDescriptorIssueActionability, PluginDescriptorIssueKind,
    PluginDescriptorIssueSeverity, PluginDescriptorKind, PluginDescriptorRepairAction,
    PluginDescriptorTargetKind,
};
pub use diagnostics::{PluginDescriptorCounts, PluginHostDiagnostics, PluginReloadReport};
pub use discovery::{
    DiscoveredPluginRoot, PluginDiscovery, PluginDiscoveryOptions, PluginDiscoverySearchRoot,
    PluginRootKind, WorkflowDirectory,
};
pub use error::PluginHostError;
pub use hook::{HookCallback, HookInvocation, HookValue};
pub use host::PluginHost;
pub use registration::{InternalPlugin, PluginActivation, PluginFinalizer, PluginRegistration};
pub use resource_registration::{
    DiscoveredPluginReload, discovered_resource_plugin_host, register_discovered_resource_plugins,
    reload_discovered_resource_plugin_host,
};
pub use runtime::{PluginRuntime, RuntimeDescriptor, UiSlotKey, UiWidgetRuntimeKey};
pub use state_cache::{
    CachedDiscoveredPluginState, cached_discovered_resource_plugin_state,
    clear_discovered_plugin_state_cache_for_tests, reload_cached_discovered_resource_plugin_state,
};
pub use status::{
    PluginErrorPhase, PluginErrorReport, PluginHealthSummary, PluginHostSnapshot,
    PluginStatusEntry, PluginStatusKind,
};
