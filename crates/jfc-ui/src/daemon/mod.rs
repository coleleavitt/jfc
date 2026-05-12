#![allow(dead_code, unused_imports, unused_variables)]
//! Fleet daemon — persistent headless agent management.
//!
//! Implements a background daemon process that manages multiple jfc sessions:
//! - Daemonize (write PID file, detach)
//! - Session registry (track active/idle/completed sessions)
//! - Cron scheduling (periodic task execution)
//! - Health monitoring (heartbeat, stall detection)
//! - Scheduled wakeups (one-shot reminders that re-fire after restarts)
//!
//! # Module layout
//!
//! - `state` — on-disk schema (`DaemonState`, `DaemonPaths`, load/save)
//! - `cron` — schedule parsing, firing logic, `run_cron_command`
//! - `pid` — PID file + `/proc` worker discovery
//! - `logs` — per-agent log file helpers
//! - `worker` — detached worker spawn + worker entry point
//! - `registry` — background-agent CRUD/queries/wait/attach
//! - `reconcile` — periodic Running-agent reconciliation/respawn
//! - `runtime` — `Daemon` struct + `run_daemon` cron loop + CLI commands
//!
//! # Storage layout
//!
//! - PID file:   `~/.config/jfc/daemon.pid`
//! - State file: `~/.config/jfc/daemon-state.json`
//! - Log dir:    `~/.config/jfc/logs/daemon/`
//!
//! # CLI
//!
//! ```bash
//! jfc daemon start           # Fork to background, write PID, run cron loop
//! jfc daemon stop            # Send SIGTERM to PID file
//! jfc daemon status          # Show daemon + session status
//! jfc daemon list            # List cron jobs + scheduled wakeups
//! jfc daemon fire <id>       # Manually fire a cron job by id
//! jfc daemon agents          # List background agents
//! jfc daemon logs <id>       # Tail a background agent's log
//! jfc daemon attach <id>     # Follow a background agent's log live
//! jfc daemon wait <id>       # Block until a background agent terminates
//! jfc daemon kill <id>       # Request cancellation of a background agent
//! jfc daemon worker --launch # Internal entry point for detached workers
//! ```

mod cron;
mod logs;
mod pid;
mod reconcile;
mod registry;
mod runtime;
mod state;
mod worker;

#[cfg(test)]
mod tests;

// ─────────────────────────────────────────────────────────────────────────────
// Public API surface (used by event_loop, stream, main, tools/subagent, …)
// ─────────────────────────────────────────────────────────────────────────────

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
    ScheduledWakeup, SessionId, SessionInfo, SessionStatus, load_state, save_state,
};
pub use worker::{run_background_agent_worker, spawn_background_agent_worker};
