use crate::runtime::StreamRequestMetadata;
use jfc_provider::StreamOptions;

pub struct PreparedStreamRequest {
    pub opts: StreamOptions,
    pub system_prompt_tokens: usize,
    pub metadata: StreamRequestMetadata,
    /// Byte length of the memory-recall block injected into the system prompt
    /// this turn (0 = no recall). Surfaced to the user as "recalled memory".
    pub recalled_memory_chars: usize,
}
