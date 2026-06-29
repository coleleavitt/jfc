//! Library facet of `jfc` — kept only for the integration tests under
//! `crates/jfc/tests/`. The engine modules they exercise live in the
//! jfc-engine crate since the stage-5 extraction; these are plain
//! re-exports (the old `#[path]` double-compile hack is gone).

pub use jfc_engine::{atomic_write, plan, plan_dreamer, plan_recall};
