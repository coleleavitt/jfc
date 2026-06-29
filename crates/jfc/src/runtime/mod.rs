pub(crate) mod dashboard;
pub(crate) mod event_loop;
mod terminal;
pub(crate) mod timeline;
mod yank;

// Engine-side runtime surface re-exported so historical `crate::runtime::X`
// paths keep working until the stage-6 shim removal. Explicit (not a glob)
// because the bin keeps its own `event_loop` module for the pump.
pub use jfc_engine::runtime::{
    APP_EVENT_BUFFER, CompactionEvent, ControlEvent, DEFERRED_TOOL_USES_CAP, DeferredToolUse,
    EngineEvent, EventReceiver, EventSender, ExecutionResult, FrontendDirective, FrontendEvent,
    GoalEvent, MessageQueue, ProviderEvent, QueuePriority, QueuedPrompt, StreamEvent,
    StreamLifecyclePhase, StreamLifecycleStatus, StreamRequestMetadata, StreamRequestOverrides,
    StreamToolChoice, TOOL_USE_SUMMARIES_CAP, TaskEvent, TeamEvent, ToolEvent, ToolProvenance,
    ToolSource, ToolUseSummary, VoiceEvent, WorkflowProgressEvent, approvals, bootstrap,
    dispatch_goal_evaluator_if_active, drain_queued_prompts, durations, factory_mode_enabled,
    handle_engine_event, handle_goal_verdict, maybe_continue_task_factory, ops,
    record_network_recovery, restart_stream_in_place, restore_persistent_background_agents,
    send_critical, sync_detached_background_tasks_from_daemon, task_drift_reminder,
    update_task_activities,
};

pub use event_loop::{AppEvent, UiEvent};

pub(crate) use terminal::{draw_synchronized, read_git_branch_from_root, set_terminal_title};
pub(crate) use yank::{
    copy_to_clipboard, full_transcript_text, last_assistant_text, tail_transcript_text,
};
