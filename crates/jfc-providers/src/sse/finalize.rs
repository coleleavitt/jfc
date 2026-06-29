use jfc_provider::{StopReason, StreamEvent};

use super::BlockState;

/// Emit terminal `*Done` events for still-open **display** blocks (text /
/// thinking) when the stream aborts mid-flight — a parse error or byte-stream
/// failure in the middle of a turn — so accumulated text isn't silently
/// dropped: a `content_block_stop` was never going to arrive, so we synthesise
/// it. The matching `ContentBlockStop` translate arm produces byte-identical
/// Done payloads to the happy path.
///
/// Open **tool-use** blocks are intentionally *not* finalized: their
/// `input_json` is partial on an abort, and emitting a `ToolDone` would
/// dispatch a partial (malformed) tool call. They're dropped here so the
/// turn errors cleanly and the retry re-issues the full call. All blocks are
/// drained either way so a later finalize pass can't double-emit.
pub fn finalize_open_blocks(
    blocks: &mut Vec<Option<BlockState>>,
    stop_reason: &mut Option<StopReason>,
) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    for (index, slot) in blocks.iter_mut().enumerate() {
        match slot.take() {
            Some(BlockState::Text { accumulated }) if !accumulated.is_empty() => {
                out.push(StreamEvent::TextDone {
                    index,
                    text: accumulated,
                });
            }
            Some(BlockState::Thinking {
                accumulated,
                signature,
                ..
            }) if !accumulated.is_empty() => {
                out.push(StreamEvent::ThinkingDone {
                    index,
                    text: accumulated,
                    signature,
                });
            }
            Some(BlockState::RedactedThinking { data }) => {
                out.push(StreamEvent::RedactedThinkingDone { index, data });
            }
            // Open tool blocks (partial input) and already-empty/None blocks
            // are dropped — see the doc comment.
            _ => {}
        }
    }
    let _ = stop_reason;
    if !out.is_empty() {
        tracing::warn!(
            target: "jfc::provider::anthropic_sse",
            finalized = out.len(),
            "finalized open display blocks after mid-stream abort to avoid losing committed text"
        );
    }
    out
}
