//! Swarm / Team orchestration for jfc.
//!
//! Implements the multi-agent "swarm" system from Claude Code v126+:
//!
//! - **Teams**: Named groups with a leader and multiple teammates
//! - **Teammates**: Long-lived named agents that persist between turns, communicate
//!   via mailbox messaging, and coordinate via a shared task list
//! - **Mailbox**: DB-backed message delivery keyed by team and agent
//! - **Permission sync**: Workers forward permission prompts to the team leader
//! - **In-process runner**: Agent loop for teammates running in the same process
//!
//! Architecture mirrors v126 `src/utils/swarm/`:
//!
//! ```text
//! session_artifacts("__swarm__", "team_file", team) — team roster metadata
//! agent_mailbox rows keyed by team + agent — teammate communication
//! session_artifacts("__swarm__", "permission_*", team/request) — approvals
//! session_artifacts("__task_store__:*", "task_store", "root") — shared tasks
//! ```

pub mod constants;
pub mod coordinator;
pub mod dispatch;
pub mod executor;
pub mod mailbox;
pub mod permission_sync;
pub mod process_bridge_teammate;
pub mod process_bridge_teammate_events;
mod process_bridge_teammate_host_requests;
mod process_bridge_teammate_io;
mod process_bridge_teammate_loop;
pub mod runner;
pub mod spawn_lifecycle;
pub mod team_helpers;
pub mod teleport;
pub mod types;

pub use constants::*;
pub use types::*;

#[cfg(test)]
pub mod test_support;

#[cfg(test)]
mod process_bridge_teammate_test_support;

#[cfg(test)]
mod process_bridge_teammate_host_requests_tests;

#[cfg(test)]
mod process_bridge_teammate_tests;
