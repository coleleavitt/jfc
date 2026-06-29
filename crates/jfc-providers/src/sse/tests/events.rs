use super::*;

#[test]
fn translate_error_event() {
    let (mut blocks, mut sr) = empty_state();
    let out = translate(
        SseEvent::Error {
            error: ErrorBody {
                kind: None,
                message: "overloaded".into(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    assert!(matches!(out, Some(StreamEvent::Error { message }) if message == "overloaded"));
}

#[test]
fn translate_transient_error_event_requests_auto_retry() {
    let (mut blocks, mut sr) = empty_state();
    for kind in ["overloaded_error", "rate_limit_error", "api_error"] {
        let out = translate(
            SseEvent::Error {
                error: ErrorBody {
                    kind: Some(kind.into()),
                    message: "transient".into(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        assert!(
            matches!(out, Some(StreamEvent::Error { message }) if message.starts_with(crate::anthropic::AUTO_RETRY_SENTINEL)),
            "{kind}"
        );
    }
}

#[test]
fn translate_ping_emits_nothing_message_start_emits_metadata() {
    let (mut blocks, mut sr) = empty_state();
    assert!(translate(SseEvent::Ping, &mut blocks, &mut sr).is_none());
    assert!(matches!(
        translate(
            SseEvent::MessageStart {
                message: MessageStart {
                    id: "msg_1".into(),
                    usage: None,
                },
            },
            &mut blocks,
            &mut sr,
        ),
        Some(StreamEvent::ResponseMetadata { .. })
    ));
}

#[test]
fn message_start_emits_response_metadata() {
    let json = r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":10,"cache_creation_input_tokens":3,"cache_read_input_tokens":7}}}"#;
    let event: SseEvent = serde_json::from_str(json).expect("message_start usage must parse");
    let (mut blocks, mut sr) = empty_state();

    assert!(matches!(
        translate(event, &mut blocks, &mut sr),
        Some(StreamEvent::ResponseMetadata {
            response_id, ..
        }) if response_id == "msg_1"
    ));
}
