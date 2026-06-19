mod block_lifecycle;
mod block_state;
mod cache;
mod content_block;
mod delta;
mod event;
mod finalize;
mod message;
mod request;
mod stream;
mod stream_log;
mod token_estimate;
mod tools;
mod translate;

pub use block_state::BlockState;
pub use content_block::ContentBlock;
pub use delta::Delta;
pub use event::SseEvent;
pub use finalize::finalize_open_blocks;
pub use message::{ContextManagement, ErrorBody, MessageDeltaData, MessageStart, MessageUsage};
pub use request::build_messages;
pub use stream::into_event_stream;
pub use tools::{apply_anthropic_tool_schema_controls, build_tools, build_tools_with_advisor};
pub use translate::translate;

pub(crate) use block_lifecycle::{apply_content_delta, start_content_block, stop_content_block};
pub(crate) use block_state::{append_input_delta, initial_input_json};
pub(crate) use cache::cap_cache_control_breakpoints;
pub(crate) use stream_log::log_parsed_event;
pub(crate) use token_estimate::{
    estimate_signature_thinking_tokens, estimate_thinking_text_tokens,
};

#[cfg(test)]
pub(crate) use cache::count_cache_control_breakpoints;
#[cfg(test)]
pub(crate) use request::ensure_input_object;
#[cfg(test)]
pub use translate::parse_stop_reason;

#[cfg(test)]
mod tests;
