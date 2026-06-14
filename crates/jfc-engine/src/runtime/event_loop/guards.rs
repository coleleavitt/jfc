use crate::app::EngineState;
use crate::types::*;

pub const CONFIG_RELOAD_REMINDER: &str = "CLAUDE.md / agent / settings file changed since last \
     turn. The reloaded content will be reflected in the \
     next system prompt.";

pub const MCP_REFRESH_REMINDER: &str = "An MCP server announced `tools/list_changed`. The tool \
     catalog may have changed; if you were about to call a \
     specific MCP tool, re-check it exists.";

/// Borrow the message currently slated as the streaming-assistant target —
/// only if it is still an assistant. Returns `None` when:
///
///   * No stream is in progress (`streaming_assistant_idx` is None).
///   * The stored index is out of bounds (a destructive op truncated past it).
///   * The slot a previous destructive op left behind is now a `Role::User`
///     or other non-assistant message.
///
/// The third case is the safety net for bugs like Up-arrow recall removing
/// a queued user message that sits before the active streaming assistant:
/// the remove shifts later indices down by one, and without an adjustment
/// `streaming_assistant_idx` points one slot to the left — at a user
/// placeholder. Routing every push through this helper means a stale index
/// drops the part with a `warn!` instead of silently corrupting the wire
/// shape (the API then rejects the next request with "tool_use blocks can
/// only appear in assistant messages").
pub fn streaming_assistant_mut(state: &mut EngineState) -> Option<&mut ChatMessage> {
    let idx = state.streaming_assistant_idx?;
    let len = state.messages.len();
    let msg = state.messages.get_mut(idx)?;
    if msg.role != Role::Assistant {
        tracing::warn!(
            target: "jfc::stream::guard",
            idx,
            len,
            role = ?msg.role,
            "streaming_assistant_idx pointed at non-assistant — refusing mutation"
        );
        return None;
    }
    Some(msg)
}
