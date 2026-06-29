use super::*;

#[test]
fn translate_thinking_delta_accumulates() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Thinking {
                thinking: String::new(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    let out = translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::ThinkingDelta {
                thinking: "thought".into(),
                estimated_tokens: Some(42),
            },
        },
        &mut blocks,
        &mut sr,
    );
    assert!(
        matches!(out, Some(StreamEvent::ThinkingDelta { delta, estimated_tokens: Some(42), .. }) if delta == "thought")
    );
}

#[test]
fn translate_thinking_delta_without_estimate_uses_text_size_regression() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Thinking {
                thinking: String::new(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    let out = translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::ThinkingDelta {
                thinking: "abcdefgh".into(),
                estimated_tokens: None,
            },
        },
        &mut blocks,
        &mut sr,
    );
    assert!(matches!(
        out,
        Some(StreamEvent::ThinkingDelta {
            estimated_tokens: Some(2),
            ..
        })
    ));
}

#[test]
fn short_thinking_delta_without_estimate_does_not_fake_token_regression() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Thinking {
                thinking: String::new(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    let out = translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::ThinkingDelta {
                thinking: "abc".into(),
                estimated_tokens: None,
            },
        },
        &mut blocks,
        &mut sr,
    );
    assert!(matches!(
        out,
        Some(StreamEvent::ThinkingDelta {
            estimated_tokens: None,
            ..
        })
    ));
}

#[test]
fn translate_signature_delta_emits_missing_thinking_token_topup_regression() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Thinking {
                thinking: String::new(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    let _ = translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::ThinkingDelta {
                thinking: "abcd".into(),
                estimated_tokens: Some(1),
            },
        },
        &mut blocks,
        &mut sr,
    );
    let out = translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::SignatureDelta {
                signature: "abcdefghijkl".into(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    assert!(matches!(
        out,
        Some(StreamEvent::ThinkingTokens { delta: 1, .. })
    ));
}

#[test]
fn thinking_delta_estimated_tokens_clamps_oversized_value_robust() {
    let json = format!(
        r#"{{"type":"content_block_delta","index":0,"delta":{{"type":"thinking_delta","thinking":"x","estimated_tokens":{}}}}}"#,
        u64::from(u32::MAX) + 1
    );
    let event: SseEvent = serde_json::from_str(&json).expect("thinking_delta must parse");
    let SseEvent::ContentBlockDelta {
        delta: Delta::ThinkingDelta {
            estimated_tokens, ..
        },
        ..
    } = event
    else {
        panic!("expected thinking delta");
    };

    assert_eq!(estimated_tokens, Some(u32::MAX));
}

#[test]
fn signature_delta_parses_and_emits_token_topup() {
    let json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EgYbOHMuAi0"}}"#;
    let event: SseEvent = serde_json::from_str(json).expect("signature_delta must parse");
    let (mut blocks, mut sr) = empty_state();
    blocks.push(Some(BlockState::Thinking {
        accumulated: "thought".into(),
        estimated_tokens: 1,
        signature: None,
    }));
    assert!(matches!(
        translate(event, &mut blocks, &mut sr),
        Some(StreamEvent::ThinkingTokens { delta, .. }) if delta > 0
    ));
}

#[test]
fn signature_delta_round_trips_on_thinking_done_regression() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::Thinking {
                thinking: String::new(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    let _ = translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::ThinkingDelta {
                thinking: "visible thought".into(),
                estimated_tokens: Some(3),
            },
        },
        &mut blocks,
        &mut sr,
    );
    let _ = translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::SignatureDelta {
                signature: "sig_1".into(),
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

    assert!(matches!(
        out,
        Some(StreamEvent::ThinkingDone {
            text,
            signature: Some(signature),
            ..
        }) if text == "visible thought" && signature == "sig_1"
    ));
}
