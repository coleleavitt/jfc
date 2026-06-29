//! Re-export shim: `ToolContext`, `ReadDedupCache`, and the CLAUDE.md/context
//! loaders moved to `jfc_core::context` during the engine extraction. This
//! file preserves the historical `crate::context::*` paths and is deleted in
//! the final shim-removal stage.

pub use jfc_core::context::*;
