mod agent_log_parser;
pub mod approvals;
mod background;
pub mod bootstrap;
mod dispatch;
pub mod durations;
pub mod event_loop;
mod events;
mod execution;
mod factory;
mod goal_loop;
mod mission;
mod network;
pub mod ops;
pub mod prompt_rewrite_gate;
mod provider_bridge_codec;
mod provider_bridge_events;
pub mod provider_descriptors;
mod provider_process_bridge;
mod queue;
mod runtime_action;
mod services;
pub mod session_save;
mod stream_control;
mod task_activity;

pub use background::{
    persist_background_result, restore_persistent_background_agents,
    sync_detached_background_tasks_from_daemon,
};
pub use dispatch::{FrontendDirective, handle_engine_event};
pub use events::{
    APP_EVENT_BUFFER, CompactionEvent, ControlEvent, EngineEvent, EventReceiver, EventSender,
    FrontendEvent, GoalEvent, PromptSubmission, ProviderEvent, StreamEvent, StreamLifecyclePhase,
    StreamLifecycleStatus, StreamRequestMetadata, StreamRequestOverrides, StreamToolChoice,
    TaskEvent, TeamEvent, ToolEvent, VoiceEvent, WorkflowProgressEvent, scoped_stream_sender,
    send_critical, stream_event,
};
pub use execution::{ExecutionResult, ToolErrorCategory, ToolProvenance, ToolSource};
pub use factory::{factory_mode_enabled, maybe_continue_task_factory};
pub use goal_loop::{
    cancel_goal_evaluator, dispatch_goal_evaluator_if_active, handle_goal_verdict,
};
pub use jfc_core::{
    DEFERRED_TOOL_USES_CAP, DeferredToolUse, MessageQueue, QueuePriority, QueuedPrompt,
    TOOL_USE_SUMMARIES_CAP, ToolUseSummary, push_bounded_drop_oldest,
};
#[cfg(test)]
pub use jfc_core::{DiagnosticLevel, ToolOutcome};
pub use network::record_network_recovery;
pub use queue::drain_queued_prompts;
pub use runtime_action::{
    EngineRuntimeAction, FrontendHostActionRequest, FrontendOpenPanelRequest,
    FrontendPanelFocusRequest, FrontendWidgetFocusRequest, RuntimeActionBoundaryError,
    RuntimeActionFrontendDirective, RuntimeActionOutcome, RuntimeActionSource,
    resolve_runtime_action,
};
pub use services::{
    AgentRuntime, ContextAssembler, PluginRuntime, ProviderModelResolution, ProviderRegistry,
    ProviderRegistryError, RuntimeDiagnostics, RuntimeDiagnosticsSnapshot,
    RuntimeDiagnosticsStatus, RuntimePolicy, RuntimeService, RuntimeServiceKind, RuntimeServices,
    RuntimeServicesBuilder, RuntimeServicesError, ToolRuntime, ToolRuntimeCatalogEntry,
    ToolRuntimeRequest,
};
pub use stream_control::{
    materialize_terminal_transcript_boundary, restart_stream_in_place, spawn_stream_response_scoped,
};
pub use task_activity::{task_drift_reminder, update_task_activities};
