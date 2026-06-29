//! Self-hosted worker bridge for JFC.
//!
//! This crate implements the deployable REST/SSE control plane that mirrors
//! the useful parts of Claude Code's hosted `/bridge` + `/worker` lifecycle:
//! worker bootstrap, worker state registration, heartbeats, event upload,
//! internal event queues, delivery acknowledgements, and session event
//! streaming. Storage is trait-backed; the default store is in-memory so the
//! server can run locally without infrastructure, and sqlite/Postgres can be
//! added behind the same `BridgeStore` contract.

pub mod client;
pub mod model;
pub mod server;
pub mod store;
pub mod time;
pub mod token;

pub use client::BridgeClient;
pub use model::*;
pub use server::{BridgeConfig, BridgeState, router, serve};
pub use store::{BridgeStore, MemoryBridgeStore, StoreError};
