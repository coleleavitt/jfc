mod events;
mod impls;
mod permissions;
mod recent_models;
pub mod shell_safety;
mod state;

pub use crate::runtime::{QueuePriority, QueuedPrompt};
pub use events::AppEvent;
pub use permissions::{
    ApprovalChoice, PendingApproval, PendingQuestion, PermissionDecision, PermissionMode,
    QuestionItem, QuestionOption,
};
pub use recent_models::{load_recent_models, push_recent_model};
pub use state::{
    ANIM_TICK_MS, App, BackgroundTask, ExpandedView, IDLE_TICK_MS, NetworkRecoveryProvider,
    NetworkRecoveryReason, NetworkRecoveryStatus, PromptSearch, SPINNER,
    STREAM_WATCHDOG_TIMEOUT_SECS, SelectKind, SelectRequest, TOKEN_HISTORY_CAP, TextSelection,
    TranscriptSearch,
};

#[cfg(test)]
mod tests;
