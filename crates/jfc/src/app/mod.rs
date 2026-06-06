mod engine_state;
mod events;
mod impls;
mod permissions;
mod recent_models;
pub mod shell_safety;
mod state;

pub use crate::runtime::{QueuePriority, QueuedPrompt};
pub use events::EngineEvent;
pub use permissions::{
    ApprovalChoice, PendingApproval, PendingQuestion, PermissionDecision, PermissionMode,
    QuestionItem, QuestionOption,
};
pub use recent_models::{load_recent_models, push_recent_model};
pub use engine_state::{
    BackgroundTask, EngineEffect, EngineState, NetworkRecoveryProvider, NetworkRecoveryReason,
    NetworkRecoveryStatus,
};
pub use state::{
    ANIM_TICK_MS, App, ExpandedView, IDLE_TICK_MS, PromptSearch, SPINNER,
    STREAM_WATCHDOG_TIMEOUT_SECS, SelectKind, SelectRequest, TOKEN_HISTORY_CAP, TextSelection,
    TranscriptSearch,
};

#[cfg(test)]
mod tests;
