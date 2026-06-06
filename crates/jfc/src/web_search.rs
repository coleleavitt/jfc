//! Web search — delegated to the `jfc-web` crate.
//!
//! This module re-exports the public API so existing `crate::web_search::search`
//! call sites continue to compile without modification.

pub use jfc_web::search;
