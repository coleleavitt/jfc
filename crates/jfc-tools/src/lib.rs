//! Pure tool implementations extracted from `jfc-ui`.
//!
//! This crate provides the core logic for tools that don't need access to
//! the TUI application state, event loop, or runtime. The dispatch
//! orchestration (permissions, caching, slop guard) remains in `jfc-ui`.

pub mod filesystem;
pub mod notebook;

pub use jfc_core::{ExecutionResult, ToolOutcome};
