//! Fleet daemon — persistent headless agent management.
//!
//! This crate handles daemon lifecycle, cron scheduling, PID management,
//! background agent state persistence, and log management. The actual
//! agent execution (providers, tools, worktrees) lives in `jfc-ui` which
//! spawns worker processes that call back into the runtime.

pub mod cron;
pub mod logs;
pub mod pid;
pub mod reconcile;
pub mod registry;
pub mod runtime;
pub mod state;
pub mod worker;

#[cfg(test)]
mod tests;

pub use cron::{CronField, CronJob, CronSchedule, parse_schedule, should_fire_cron};
pub use logs::read_last_lines;
pub use pid::{is_daemon_running, remove_pid_file, write_pid_file};
pub use registry::{
    attach_background_agent_cli, background_agent_cancel_requested, background_agent_logs_string,
    background_agents_for_restore, background_agents_string, record_background_agent_finished,
    record_background_agent_log, record_background_agent_progress, record_background_agent_started,
    request_background_agent_cancel, wait_background_agent_cli,
};
pub use runtime::{Daemon, fire_cron_cli, list_string, run_daemon, status_string, stop_daemon};
pub use state::{
    BackgroundAgentInfo, BackgroundAgentLaunch, BackgroundAgentStatus, DaemonPaths, DaemonState,
    ScheduledWakeup, SessionId, SessionInfo, SessionStatus, TERMINAL_AGENT_CAP,
    TERMINAL_AGENT_RETENTION, compact_background_agents, load_state, load_state_if_changed,
    save_state, state_file_mtime,
};
pub use worker::{spawn_background_agent_worker};
