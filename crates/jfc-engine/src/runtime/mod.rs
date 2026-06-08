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
mod network;
pub mod ops;
mod queue;
mod stream_control;
mod task_activity;

pub use background::{
    restore_persistent_background_agents, sync_detached_background_tasks_from_daemon,
};
pub use dispatch::{FrontendDirective, handle_engine_event};
pub use events::{
    APP_EVENT_BUFFER, CompactionEvent, ControlEvent, EngineEvent, EventReceiver, EventSender,
    FrontendEvent, GoalEvent, ProviderEvent, StreamEvent, StreamLifecyclePhase,
    StreamLifecycleStatus, StreamRequestMetadata, StreamRequestOverrides, StreamToolChoice,
    TaskEvent, TeamEvent, ToolEvent, VoiceEvent, WorkflowProgressEvent, send_critical,
};
pub use execution::{ExecutionResult, ToolProvenance, ToolSource};
pub use factory::{factory_mode_enabled, maybe_continue_task_factory};
pub use goal_loop::{dispatch_goal_evaluator_if_active, handle_goal_verdict};
pub use jfc_core::{
    DEFERRED_TOOL_USES_CAP, DeferredToolUse, MessageQueue, QueuePriority, QueuedPrompt,
    TOOL_USE_SUMMARIES_CAP, ToolUseSummary,
};
#[cfg(test)]
pub use jfc_core::{DiagnosticLevel, ToolOutcome};
pub use network::record_network_recovery;
pub use queue::drain_queued_prompts;
pub use stream_control::restart_stream_in_place;
pub use task_activity::{task_drift_reminder, update_task_activities};
