mod events;
mod impls;
mod permissions;
mod recent_models;
mod state;

pub use events::AppEvent;
pub use permissions::{ApprovalChoice, PendingApproval, PermissionDecision, PermissionMode};
pub use recent_models::{load_recent_models, push_recent_model, save_recent_models};
pub use state::{
    ANIM_TICK_MS, App, BackgroundTask, IDLE_TICK_MS, NetworkRecoveryProvider,
    NetworkRecoveryReason, NetworkRecoveryStatus, QueuedPrompt, SPINNER,
    STREAM_WATCHDOG_TIMEOUT_SECS, TOKEN_HISTORY_CAP, TranscriptSearch,
};

#[cfg(test)]
mod tests;
