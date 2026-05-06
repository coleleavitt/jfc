//! Swarm / Team orchestration for jfc.
//!
//! Implements the multi-agent "swarm" system from Claude Code v126+:
//!
//! - **Teams**: Named groups with a leader and multiple teammates
//! - **Teammates**: Long-lived named agents that persist between turns, communicate
//!   via mailbox messaging, and coordinate via a shared task list
//! - **Mailbox**: File-based message delivery with JSON inbox files and file locking
//! - **Permission sync**: Workers forward permission prompts to the team leader
//! - **In-process runner**: Agent loop for teammates running in the same process
//!
//! Architecture mirrors v126 `src/utils/swarm/`:
//!
//! ```text
//! ~/.claude/teams/{team-name}/
//!   config.json          — team file (members, lead, metadata)
//!   inboxes/             — per-agent mailbox files
//!     researcher.json
//!     team-lead.json
//!   permissions/
//!     pending/           — permission requests awaiting leader approval
//!     resolved/          — completed permission decisions
//!
//! ~/.claude/tasks/{team-name}/
//!   tasks.json           — shared task list all teammates can access
//! ```

pub mod constants;
pub mod mailbox;
pub mod permission_sync;
pub mod runner;
pub mod team_helpers;
pub mod types;

pub use constants::*;
pub use types::*;

#[cfg(test)]
pub(crate) mod test_support;
