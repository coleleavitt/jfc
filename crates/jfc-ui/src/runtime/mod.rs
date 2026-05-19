mod agent_log_parser;
mod background;
mod events;
mod execution;
mod factory;
mod goal_loop;
mod network;
pub(crate) mod network_ekg;
mod queue;
mod stream_control;
mod task_activity;
mod terminal;
mod yank;

pub(crate) use background::{
    restore_persistent_background_agents, sync_detached_background_tasks_from_daemon,
};
pub use events::{
    APP_EVENT_BUFFER, AppEvent, CompactionEvent, EventReceiver, EventSender, GoalEvent,
    ProviderEvent, StreamEvent, TaskEvent, TeamEvent, ToolEvent, UiEvent,
};
pub use execution::{
    DiagnosticLevel, ExecutionResult, ToolDiagnostic, ToolOutcome, ToolProvenance, ToolSource,
};
pub(crate) use factory::{factory_mode_enabled, maybe_continue_task_factory};
pub(crate) use goal_loop::{dispatch_goal_evaluator_if_active, handle_goal_verdict};
pub(crate) use network::record_network_recovery;
pub(crate) use queue::drain_queued_prompts;
pub(crate) use stream_control::restart_stream_in_place;
pub(crate) use task_activity::update_task_activities;
pub(crate) use terminal::{draw_synchronized, read_git_branch_from_root, set_terminal_title};
pub(crate) use yank::{
    copy_to_clipboard, full_transcript_text, last_assistant_text, tail_transcript_text,
    yank_last_assistant,
};
