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
/// Stream watchdog default idle timeout (seconds) — see `check_stream_watchdog`.
///
/// This is a *coarse backstop*, not the primary liveness mechanism. jfc has two
/// layered idle clocks:
///   1. Byte-level (jfc-anthropic-sdk `byte_stream_events`): wraps the raw
///      socket in a per-chunk timeout (default 600s) that resets on every byte.
///   2. Event-level (this watchdog): resets `last_stream_event_at` on every
///      decoded stream event *and* on content-free `Keepalive` ticks (SSE
///      pings), so an actively-streaming response — including a long
///      tool-input/file write or an extended-thinking pause — keeps itself
///      alive without producing visible deltas.
///
/// Because both clocks now reset on genuine wire activity, this threshold only
/// fires when the stream is *truly* silent (no bytes, no pings) for the whole
/// window. It was 90s, which was both stricter than Claude Code's 300s idle
/// deadline and *below* our own 600s byte timeout — so the event watchdog could
/// cancel a slow-but-alive stream before the byte layer ever would. 180s is a
/// sane silence backstop that sits above normal inter-event cadence yet still
/// reaps a genuinely dead connection promptly. Override with
/// `JFC_STREAM_WATCHDOG_TIMEOUT_SECS=<secs>` or disable with
/// `JFC_DISABLE_STREAM_WATCHDOG=1`.
///
/// This is the *responding/idle-model* base — the aggressive tier. See
/// `STREAM_WATCHDOG_THINKING_TIMEOUT_SECS` for the lenient extended-thinking
/// tier, and `check_stream_watchdog` for how the phase is selected.
pub const STREAM_WATCHDOG_TIMEOUT_SECS: u64 = 180;

/// Stream watchdog idle timeout (seconds) while the model is **actively
/// thinking** (an extended-thinking block is open: `thinking_started_at` set,
/// `thinking_ended_at` not yet). Extended / summarized / redacted thinking can
/// go genuinely byte-quiet for long stretches — the server is reasoning, not
/// streaming — and a premature cancel there throws away the most expensive part
/// of the turn. So the thinking tier is deliberately lenient.
///
/// Conversely, once the model is *responding* (emitting text/tool deltas) or is
/// a non-thinking model that never opens a thinking block, a silent wire almost
/// certainly means a dead socket, so the tighter `STREAM_WATCHDOG_TIMEOUT_SECS`
/// applies and we reap it fast. Keepalive pings and thinking-token estimates
/// already reset the idle clock, so this lenient tier only ever governs a truly
/// silent thinking pause. Override with `JFC_STREAM_WATCHDOG_THINKING_TIMEOUT_SECS`.
pub const STREAM_WATCHDOG_THINKING_TIMEOUT_SECS: u64 = 600;

pub use engine_state::{
    BackgroundAgentCompletion, BackgroundTask, BackgroundTaskActivity, BackgroundTaskActivityKind,
    EngineEffect, EngineState, MAX_NETWORK_RECOVERY_ATTEMPTS, NetworkRecoveryProvider,
    NetworkRecoveryReason, NetworkRecoveryStatus,
};
pub use events::EngineEvent;
pub use permissions::{
    ApprovalChoice, BuiltinRuntimePolicy, PendingApproval, PendingQuestion, PermissionDecision,
    PermissionMode, QuestionItem, QuestionOption,
};
pub use recent_models::{load_recent_models, push_recent_model};
