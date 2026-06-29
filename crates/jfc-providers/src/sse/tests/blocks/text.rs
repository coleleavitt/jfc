use super::*;

#[test]
fn translate_text_block_lifecycle() {
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
    assert!(matches!(blocks[0], Some(BlockState::Text { .. })));

    let out = translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::TextDelta {
                text: "chunk1".into(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    assert!(matches!(out, Some(StreamEvent::TextDelta { delta, .. }) if delta == "chunk1"));

    translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::TextDelta {
                text: "chunk2".into(),
            },
        },
        &mut blocks,
        &mut sr,
    );

    let out = translate(
        SseEvent::ContentBlockStop { index: 0 },
        &mut blocks,
        &mut sr,
    );
    assert!(matches!(out, Some(StreamEvent::TextDone { text, .. }) if text == "chunk1chunk2"));
    assert!(blocks[0].is_none());
}

#[test]
fn translate_block_stop_missing_index() {
    let (mut blocks, mut sr) = empty_state();
    assert!(
        translate(
            SseEvent::ContentBlockStop { index: 99 },
            &mut blocks,
            &mut sr
        )
        .is_none()
    );
}

#[test]
fn translate_multi_block_indices_independent() {
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
        SseEvent::ContentBlockStart {
            index: 1,
            content_block: ContentBlock::Thinking {
                thinking: String::new(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::TextDelta { text: "a".into() },
        },
        &mut blocks,
        &mut sr,
    );
    translate(
        SseEvent::ContentBlockDelta {
            index: 1,
            delta: Delta::ThinkingDelta {
                thinking: "t".into(),
                estimated_tokens: None,
            },
        },
        &mut blocks,
        &mut sr,
    );
    let t0 = translate(
        SseEvent::ContentBlockStop { index: 0 },
        &mut blocks,
        &mut sr,
    );
    let t1 = translate(
        SseEvent::ContentBlockStop { index: 1 },
        &mut blocks,
        &mut sr,
    );
    assert!(matches!(t0, Some(StreamEvent::TextDone { text, .. }) if text == "a"));
    assert!(matches!(t1, Some(StreamEvent::ThinkingDone { text, .. }) if text == "t"));
}
