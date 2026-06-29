//! Fleet daemon — persistent headless agent management.
//!
//! Core daemon logic (cron, pid, state, logs, registry, reconcile, runtime,
//! worker spawn) lives in the `jfc-daemon` crate. This module re-exports
//! everything and adds the worker execution entry point that depends on
//! the full jfc runtime (providers, tools, worktrees).

mod background_worktree;
pub mod worker;
mod worker_mcp;
mod worker_state;

// Re-export the full jfc-daemon public API so existing callsites keep working.
pub use jfc_daemon::*;

// The worker execution entry point (run_background_agent_worker) stays here
// because it depends on crate::build_providers, crate::tools, crate::worktrees.
pub use worker::{run_background_agent_worker, spawn_background_agent_worker};
