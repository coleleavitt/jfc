//! Pure tool implementations extracted from `jfc`.
//!
//! This crate provides the core logic for tools that don't need access to
//! the TUI application state, event loop, or runtime. The dispatch
//! orchestration (permissions, caching, slop guard) remains in `jfc`.

pub mod bash_processes;
pub mod filesystem;
pub mod notebook;

pub use jfc_core::{ExecutionResult, ToolOutcome};
