mod impls;
mod input_state;
mod panel_state;
mod plugin_panel_refresh;
mod plugin_panel_refresh_policy;
#[cfg(test)]
mod plugin_panel_refresh_tests;
mod plugin_panel_state;
mod plugin_runtime_extension_state;
mod plugin_status;
#[cfg(test)]
mod plugin_status_tests;
mod plugin_widget_bridge;
mod plugin_widget_refresh;
mod plugin_widget_refresh_policy;
#[cfg(test)]
mod plugin_widget_refresh_tests;
mod plugin_widget_state;
mod state;

// Engine-side app types re-exported from jfc-engine so the historical
// `crate::app::X` paths keep working until the stage-6 shim removal.
pub use input_state::{
    BashPickerState, CommandPaletteState, ModelPickerState, SessionPickerState, ThemePickerState,
};
pub use jfc_engine::app::{
    ApprovalChoice, BACKGROUND_REMINDERS_CAP, BackgroundTask, DEFAULT_CONTEXT_WINDOW_TOKENS,
    EngineEffect, EngineEvent, EngineState, NetworkRecoveryProvider, NetworkRecoveryReason,
    NetworkRecoveryStatus, PendingApproval, PendingQuestion, PermissionDecision, PermissionMode,
    QuestionItem, QuestionOption, STREAM_WATCHDOG_TIMEOUT_SECS, TOKEN_HISTORY_CAP,
    load_recent_models, push_recent_model, shell_safety,
};
pub use jfc_engine::runtime::{QueuePriority, QueuedPrompt};
pub use panel_state::{
    ExpandedView, FocusedUiPanel, FocusedUiWidget, InfoSidebarState, SessionSidebarState,
    TaskPanelUiState,
};
#[cfg(test)]
pub(crate) use plugin_panel_state::{UiPanelRefreshStatus, UiPanelSnapshot};
pub(crate) use plugin_panel_state::{
    UiPanelRefreshStatuses, UiPanelSnapshots, ui_panel_snapshot_key,
};
#[cfg(test)]
pub(crate) use plugin_widget_state::{UiWidgetRefreshStatus, UiWidgetSnapshot};
pub(crate) use plugin_widget_state::{
    UiWidgetRefreshStatuses, UiWidgetSnapshots, ui_widget_snapshot_key,
};
pub use state::{
    ANIM_TICK_MS, App, IDLE_TICK_MS, PromptRewriteProposal, PromptSearch, SPINNER, SelectKind,
    SelectRequest, TextSelection, TranscriptSearch, VOICE_AUDIO_LEVELS_CAP, VOICE_TTS_TIMINGS_CAP,
};

#[cfg(test)]
mod tests;
