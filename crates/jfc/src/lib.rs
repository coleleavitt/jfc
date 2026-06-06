//! Library facet of `jfc` — exposes the small set of in-process modules
//! that integration tests (under `crates/jfc/tests/`) need to reach.
//!
//! The bulk of the crate is still the `jfc` binary (see `src/main.rs`); this
//! library is intentionally minimal and re-uses the *same* source files via
//! `#[path]` attributes so we don't double-maintain code.
//!
//! Modules listed here are duplicated at compile time (once by `main.rs`,
//! once by `lib.rs`). Keep the surface area tight — only add a module here
//! when a test or downstream crate actually needs it.

#[path = "atomic_write.rs"]
pub mod atomic_write;

#[path = "plan.rs"]
pub mod plan;

#[path = "plan_dreamer.rs"]
pub mod plan_dreamer;

#[path = "plan_recall.rs"]
pub mod plan_recall;
