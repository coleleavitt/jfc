mod impls;
mod state;

// Engine-side app types re-exported from jfc-engine so the historical
// `crate::app::X` paths keep working until the stage-6 shim removal.
pub use jfc_engine::app::{
    ApprovalChoice, BACKGROUND_REMINDERS_CAP, BackgroundTask, DEFAULT_CONTEXT_WINDOW_TOKENS,
    EngineEffect, EngineEvent, EngineState, NetworkRecoveryProvider, NetworkRecoveryReason,
    NetworkRecoveryStatus, PendingApproval, PendingQuestion, PermissionDecision, PermissionMode,
    QuestionItem, QuestionOption, STREAM_WATCHDOG_TIMEOUT_SECS, TOKEN_HISTORY_CAP,
    load_recent_models, push_recent_model, shell_safety,
};
pub use jfc_engine::runtime::{QueuePriority, QueuedPrompt};
pub use state::{
    ANIM_TICK_MS, App, ExpandedView, IDLE_TICK_MS, PromptRewriteProposal, PromptSearch, SPINNER,
    SelectKind, SelectRequest, TextSelection, TranscriptSearch,
};

#[cfg(test)]
mod tests;
