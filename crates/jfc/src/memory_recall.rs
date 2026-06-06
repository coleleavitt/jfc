//! Two-phase LLM-driven memory recall — delegated to the `jfc-memory` crate.
//!
//! Re-exports the public API so existing `crate::memory_recall::*` call sites
//! continue to compile without modification.

pub use jfc_memory::recall::{cached_recall, is_enabled, run_recall, set_runtime_override};
