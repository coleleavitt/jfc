//! `jfc-changeset` — durable, reviewable, reversible agent change-sets.
//!
//! Borrows Dolt's "every agent gets an isolated branch that is reviewed and
//! tested before it touches production" model and makes it a first-class
//! runtime object. Every mutating agent run becomes an [`AgentChangeSet`]: a
//! durable record pinning the git isolation (base head, branch, worktree), the
//! produced diff, an attached test/approval trail, and a lifecycle state.
//!
//! The lifecycle is an explicit state machine
//! ([`ChangeState`]): `Draft → Ready → Tested → Approved → Applied`, plus
//! `Reverted` (undo an applied change) and `Abandoned` (discard before apply).
//! The ordering is the safety invariant — a change **cannot** reach `Applied`
//! (touch the main checkout) without passing through `Tested` then `Approved`,
//! so "reviewed and tested before production" is enforced by construction, not
//! convention.
//!
//! [`ChangeStore`] persists change-sets to an append-only JSONL file under
//! `.jfc/changes/`, using the same flock discipline as `jfc-audit`'s finding
//! store, so the history is queryable across processes and survives restarts.
//!
//! This crate is intentionally dependency-light (no `jfc-ui`, `jfc-graph`, or
//! `jfc-economy`) so the lifecycle model can be reused by the UI, the daemon,
//! and economy mode without a dependency cycle.

mod error;
mod state;
mod store;
mod types;

pub use error::{ChangeSetError, Result};
pub use state::ChangeState;
pub use store::{ChangeFilter, ChangeStore};
pub use types::{AgentChangeSet, Approval, ChangedFile, TestRun};
