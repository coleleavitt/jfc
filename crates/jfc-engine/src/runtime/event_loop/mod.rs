//! Engine event handlers (formerly the binary's event-loop handler set).
//! The frontend pumps live in their own crates; only the frontend-neutral
//! handlers and their shared guards moved here.

pub mod guards;
pub mod handlers;
pub mod narration_retry;
