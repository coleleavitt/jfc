//! Re-exports from `jfc-core`. The canonical definitions now live there;
//! this shim keeps existing `use crate::runtime::ExecutionResult` paths working.

pub use jfc_core::{ExecutionResult, ToolProvenance, ToolSource};
