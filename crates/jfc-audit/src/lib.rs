//! `jfc-audit` — Graph-driven recursive bug auditor.
//!
//! This crate implements a multi-phase audit pipeline:
//! 1. **Enumerate** entry points from the code graph
//! 2. **Scan** source for suspicious patterns (unsafe, unwrap, panics, etc.)
//! 3. **Prove** reachability from entry points to suspicious code
//! 4. **Trace** taint flow from sources to sinks
//! 5. **Validate** findings via the bounty economy
//! 6. **Persist** findings to an append-only store
//!
//! All external dependencies (graph, economy, LLM) are abstracted behind traits,
//! so this crate compiles standalone without pulling in jfc-graph or jfc-economy.

pub mod dispatcher;
pub mod enumerator;
pub mod error;
pub mod orchestrator;
pub mod reachability;
pub mod store;
pub mod suspicious_point;
pub mod taint;
pub mod types;

pub use error::{AuditError, Result};
pub use orchestrator::{AuditConfig, AuditOrchestrator, AuditReport, AuditStats};
pub use store::{FindingFilter, FindingStore};
pub use types::{Finding, FindingKind, Granularity, PocStatus, Severity, SourceSpan};
