use crate::context_accounting::RequestContextPressure;
use crate::runtime::StreamRequestMetadata;
use jfc_provider::StreamOptions;

/// Fully assembled provider request state for one assistant turn.
///
/// `prepare_stream_request` builds this after prompt/context assembly and
/// before streaming starts. Downstream stream execution consumes `opts` as the
/// provider-facing request, while the token counts and metadata are retained
/// for telemetry, compaction decisions, and user-visible recall indicators.
pub struct PreparedStreamRequest {
    pub opts: StreamOptions,
    pub context_pressure: RequestContextPressure,
    pub system_prompt_tokens: usize,
    pub metadata: StreamRequestMetadata,
    /// Byte length of the fresh memory-recall block injected into the system
    /// prompt this turn (0 = no fresh recall). Surfaced to the user as
    /// "recalled memory"; cached/context-store blocks are counted in budget
    /// telemetry but do not fire another toast.
    pub recalled_memory_chars: usize,
}
