//! Re-export MCP elicitation types from jfc-core.
//!
//! The implementation lives in `jfc_core::mcp_elicitation` so both
//! `jfc-mcp` (which handles the rmcp callbacks) and `jfc-engine` (which
//! owns the event routing) can access it without a circular dependency.
pub use jfc_core::mcp_elicitation::*;
