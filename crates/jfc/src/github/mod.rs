//! GitHub integrations — the interactive install wizard stays with the TUI;
//! everything else re-exports from jfc-engine until the stage-6 shim removal.

pub mod install;

pub use jfc_engine::github::*;
