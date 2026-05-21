//! Process sandboxing for economy solver agents.
//!
//! Provides path restriction and network blocking for child processes.
//! Gracefully degrades when kernel support is unavailable.

use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "linux")]
pub mod landlock;

#[cfg(not(target_os = "linux"))]
pub mod landlock {
    //! Stub for non-Linux platforms.

    pub use super::fallback::*;
}

#[cfg(not(target_os = "linux"))]
mod fallback;

pub use landlock::{SandboxPolicy, SandboxResult};

/// Global flag: set to `true` once the landlock sandbox has been
/// successfully initialized for this process. Read by the permission
/// system to auto-approve tools when sandboxed.
static SANDBOX_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Mark the sandbox as active. Call this after successful landlock
/// initialization at process startup.
pub fn mark_sandbox_active() {
    SANDBOX_ACTIVE.store(true, Ordering::Relaxed);
}

/// Returns `true` if the landlock sandbox was successfully initialized
/// for this process. Used by the permission system to skip approval
/// prompts when execution is already containerized.
pub fn is_sandbox_active() -> bool {
    SANDBOX_ACTIVE.load(Ordering::Relaxed)
}
