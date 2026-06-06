//! Engine-side remnants of the binary's `app` module: the `EngineState`
//! itself plus the approval/permission types. The path is kept (`crate::app`)
//! so the moved tree compiles unchanged; stage 6/7 flatten it.

mod engine_state;
mod events;
mod permissions;
pub mod recent_models;
pub mod shell_safety;

/// Engine-owned tuning constants (formerly in the binary's app/state.rs).
pub const DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 200_000;
/// Cap for the queued `<system-reminder>` bodies (oldest dropped first).
pub const BACKGROUND_REMINDERS_CAP: usize = 20;
/// Rolling per-turn token history kept for the frontend sparkline.
pub const TOKEN_HISTORY_CAP: usize = 32;
/// Stream watchdog default timeout (seconds) — see check_stream_watchdog.
pub const STREAM_WATCHDOG_TIMEOUT_SECS: u64 = 90;

pub use engine_state::{
    BackgroundTask, EngineEffect, EngineState, NetworkRecoveryProvider, NetworkRecoveryReason,
    NetworkRecoveryStatus,
};
pub use events::EngineEvent;
pub use permissions::{
    ApprovalChoice, PendingApproval, PendingQuestion, PermissionDecision, PermissionMode,
    QuestionItem, QuestionOption,
};
pub use recent_models::{load_recent_models, push_recent_model};
