use crate::runtime::StreamRequestMetadata;
use jfc_provider::StreamOptions;

pub struct PreparedStreamRequest {
    pub opts: StreamOptions,
    pub system_prompt_tokens: usize,
    pub metadata: StreamRequestMetadata,
    /// Byte length of the fresh memory-recall block injected into the system
    /// prompt this turn (0 = no fresh recall). Surfaced to the user as
    /// "recalled memory"; cached/context-store blocks are counted in budget
    /// telemetry but do not fire another toast.
    pub recalled_memory_chars: usize,
}
