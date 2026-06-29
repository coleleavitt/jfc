//! # jfc-agent
//!
//! Unified agent primitives shared by every execution backend in JFC.
//!
//! Before this crate, "an agent" was modelled three or four different ways
//! depending on where it ran (each row a separate, near-duplicate type):
//!
//! | Concern   | Background (engine)   | Daemon                  | Teammate (swarm)         | Economy           |
//! |-----------|-----------------------|-------------------------|--------------------------|-------------------|
//! | Identity  | `AgentId(u64)`        | `id: String`            | `agent_id: String`       | `AgentId(String)` |
//! | Status    | `AgentStatus`         | `BackgroundAgentStatus` | `TeammateStatus`         | (ad hoc)          |
//! | State     | `BackgroundTask`      | `BackgroundAgentInfo`   | `InProcessTeammateState` | pool entries      |
//! | Spawn     | `AgentConfig`         | `BackgroundAgentLaunch` | `TeammateRunnerConfig`   | charter           |
//! | Messaging | completion events     | log files               | `TeammateEvent` + mailbox| stub              |
//!
//! The fully-dead duplicates have since been deleted (the engine's old
//! `background::BackgroundManager` cluster and the swarm's
//! `InProcessTeammateState`/`TeammateStatus`/`TeammateProgress`); the economy's
//! `AgentId(String)` is now this crate's [`AgentId`]; and every spawn path
//! mirrors its lifecycle into the shared [`AgentRegistry`]. This crate is the
//! one model they all converge on:
//!
//! - [`AgentId`] — a UUID newtype with an optional display name.
//! - [`AgentStatus`] — one lifecycle enum.
//! - [`AgentRole`] — what kind of agent, carrying role-specific data.
//! - [`AgentState`] — the single record describing a running agent.
//! - [`SpawnConfig`] / [`AgentRegistry`] — the one seam backends implement.
//! - [`MessageBus`] — one delivery hub (in-memory + file-backed).
//!
//! Backend *implementations* (in-process tokio, detached daemon, team
//! coordinator, council fan-out, bounty coordinator) live in
//! `jfc-engine/src/agents/`; this crate stays dependency-light so every other
//! crate can depend on the shared vocabulary without pulling in the engine.

pub mod id;
pub mod message;
pub mod registry;
pub mod state;

pub use id::AgentId;
pub use message::{Message, MessageBus, MessageError};
pub use registry::{AgentRegistry, RegistryError, SpawnConfig};
pub use state::{AgentResult, AgentRole, AgentState, AgentStatus};
