use super::*;

#[test]
fn finalize_open_blocks_flushes_open_text_normal() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Text {
                text: String::new(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::TextDelta {
                text: "partial answer".into(),
            },
        },
        &mut blocks,
        &mut sr,
    );

    let flushed = finalize_open_blocks(&mut blocks, &mut sr);
    assert!(
        matches!(flushed.as_slice(), [StreamEvent::TextDone { index: 0, text }] if text == "partial answer"),
        "expected a single TextDone carrying the accumulated text, got {flushed:?}"
    );
    // Block is drained so a later finalize pass can't double-emit it.
    assert!(blocks[0].is_none());
}

// An open tool-use block has only partial input on an abort; finalizing it
// would dispatch a malformed call, so it is dropped (drained, not emitted).

#[test]
fn finalize_open_blocks_drops_partial_tool_robust() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::ToolUse {
                id: "tool_1".into(),
                name: "Bash".into(),
                input: serde_json::json!({}),
            },
        },
        &mut blocks,
        &mut sr,
    );
    translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::InputJsonDelta {
                partial_json: "{\"command\": \"ec".into(),
            },
        },
        &mut blocks,
        &mut sr,
    );

    let flushed = finalize_open_blocks(&mut blocks, &mut sr);
    assert!(
        flushed.is_empty(),
        "partial tool block must not be finalized into a ToolDone, got {flushed:?}"
    );
    assert!(blocks[0].is_none(), "tool block should still be drained");
}

// No open blocks → nothing to flush.

#[test]
fn finalize_open_blocks_empty_is_noop_robust() {
    let (mut blocks, mut sr) = empty_state();
    assert!(finalize_open_blocks(&mut blocks, &mut sr).is_empty());
}
