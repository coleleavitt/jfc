use super::*;

#[test]
fn parse_stop_reason_all_variants() {
    assert_eq!(parse_stop_reason(Some("end_turn")), StopReason::EndTurn);
    assert_eq!(parse_stop_reason(Some("tool_use")), StopReason::ToolUse);
    // pause_turn must NOT bucket into Other(...) — that drops it into
    // event_loop's "model said its piece" else branch and silently ends
    // the agentic loop. See StopReason::PauseTurn docs.
    assert_eq!(parse_stop_reason(Some("pause_turn")), StopReason::PauseTurn);
    assert_eq!(parse_stop_reason(Some("max_tokens")), StopReason::MaxTokens);
    assert_eq!(
        parse_stop_reason(Some("stop_sequence")),
        StopReason::StopSequence
    );
    assert_eq!(parse_stop_reason(Some("refusal")), StopReason::Refusal);
    assert_eq!(parse_stop_reason(None), StopReason::EndTurn);
}

#[test]
fn translate_message_stop_with_reason() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::MessageDelta {
            delta: MessageDeltaData {
                stop_reason: Some("end_turn".into()),
            },
            usage: None,
            context_management: None,
        },
        &mut blocks,
        &mut sr,
    );
    let out = translate(SseEvent::MessageStop, &mut blocks, &mut sr);
    assert!(matches!(
        out,
        Some(StreamEvent::Done {
            stop_reason: StopReason::EndTurn
        })
    ));
}

#[test]
fn translate_message_stop_defaults_end_turn() {
    let (mut blocks, mut sr) = empty_state();
    let out = translate(SseEvent::MessageStop, &mut blocks, &mut sr);
    assert!(matches!(
        out,
        Some(StreamEvent::Done {
            stop_reason: StopReason::EndTurn
        })
    ));
}

// Robust: `parse_stop_reason(None)` still falls back to EndTurn for
// back-compat with truncated/short-circuited streams, but the
// behavior is documented + warn-logged so the silent fallback
// doesn't hide a future variant the way it hid pause_turn for
// months. This test pins the contract: missing field → EndTurn,
// NOT panic, NOT Other(""), NOT Other("null").

#[test]
fn parse_stop_reason_none_falls_back_to_end_turn_robust() {
    assert_eq!(parse_stop_reason(None), StopReason::EndTurn);
}

// Robust: a known refusal stop reason gets a first-class variant so the UI
// can stop retry loops and show a specific diagnostic.

#[test]
fn parse_stop_reason_refusal_is_first_class_robust() {
    assert_eq!(parse_stop_reason(Some("refusal")), StopReason::Refusal);
}

// Robust: an unknown variant string buckets into Other(...) and is
// expected to surface a warn in the trace log. We can't easily
// capture the tracing event from a unit test without a
// subscriber-capture rig, but we DO pin that the variant is
// preserved verbatim so the user can grep their logs for the
// exact string Anthropic sent.

#[test]
fn parse_stop_reason_unknown_string_preserves_variant_robust() {
    assert_eq!(
        parse_stop_reason(Some("container_oom")),
        StopReason::Other("container_oom".into())
    );
    // Empty string is its own degenerate case — preserved (NOT
    // collapsed to EndTurn) so it shows up in logs as
    // `Other("")` which is grep-able.
    assert_eq!(parse_stop_reason(Some("")), StopReason::Other("".into()));
}

// Normal: a message_delta with stop_reason="pause_turn" followed by
// message_stop produces a Done{PauseTurn} — NOT Other("pause_turn"),
// which would silently fall through event_loop's dispatch ladder into
// the "model said its piece" branch and end the agentic loop. See
// StopReason::PauseTurn docs.

#[test]
fn translate_message_stop_with_pause_turn_normal() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::MessageDelta {
            delta: MessageDeltaData {
                stop_reason: Some("pause_turn".into()),
            },
            usage: None,
            context_management: None,
        },
        &mut blocks,
        &mut sr,
    );
    let out = translate(SseEvent::MessageStop, &mut blocks, &mut sr);
    assert!(matches!(
        out,
        Some(StreamEvent::Done {
            stop_reason: StopReason::PauseTurn
        })
    ));
}

#[test]
fn message_delta_without_stop_reason_does_not_overwrite_pause_turn_regression() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::MessageDelta {
            delta: MessageDeltaData {
                stop_reason: Some("pause_turn".into()),
            },
            usage: None,
            context_management: None,
        },
        &mut blocks,
        &mut sr,
    );
    translate(
        SseEvent::MessageDelta {
            delta: MessageDeltaData { stop_reason: None },
            usage: Some(MessageUsage {
                input_tokens: None,
                output_tokens: Some(42),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            }),
            context_management: None,
        },
        &mut blocks,
        &mut sr,
    );

    let out = translate(SseEvent::MessageStop, &mut blocks, &mut sr);

    assert!(matches!(
        out,
        Some(StreamEvent::Done {
            stop_reason: StopReason::PauseTurn
        })
    ));
}

#[test]
fn message_delta_usage_emits_usage_event() {
    let json = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;
    let event: SseEvent = serde_json::from_str(json).expect("message_delta usage must parse");
    let (mut blocks, mut sr) = empty_state();

    assert!(matches!(
        translate(event, &mut blocks, &mut sr),
        Some(StreamEvent::Usage {
            input_tokens: 0,
            output_tokens: 42,
            ..
        })
    ));
    assert_eq!(sr, Some(StopReason::EndTurn));
}
