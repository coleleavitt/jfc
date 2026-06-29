use super::*;
use futures::StreamExt;
use std::time::Duration;

#[test]
fn parser_handles_split_utf8_and_multiline_data_normal() {
    let mut parser = SseParser::new();
    assert!(
        parser
            .push("event: update\ndata: caf".as_bytes())
            .unwrap()
            .is_empty()
    );
    let frames = parser.push("é\ndata: ok\n\n".as_bytes()).unwrap();
    assert_eq!(
        frames,
        vec![SseFrame {
            event: "update".to_owned(),
            data: "café\nok".to_owned(),
        }]
    );
}

#[test]
fn parser_handles_crlf_split_across_chunks_robust() {
    let mut parser = SseParser::new();
    assert!(parser.push(b"data: hello\r").unwrap().is_empty());
    let frames = parser.push(b"\n\r\n").unwrap();
    assert_eq!(
        frames,
        vec![SseFrame {
            event: "message".to_owned(),
            data: "hello".to_owned(),
        }]
    );
}

#[test]
fn parser_ignores_comments_and_strips_bom_normal() {
    let mut parser = SseParser::new();
    let frames = parser
        .push(b"\xEF\xBB\xBF: comment\nevent: ping\ndata: {}\n\n")
        .unwrap();
    assert_eq!(
        frames,
        vec![SseFrame {
            event: "ping".to_owned(),
            data: "{}".to_owned(),
        }]
    );
}

#[test]
fn parser_emits_final_unterminated_event_robust() {
    let mut parser = SseParser::new();
    assert!(parser.push(b"data: [DONE]").unwrap().is_empty());
    let frames = parser.finish().unwrap();
    assert_eq!(
        frames,
        vec![SseFrame {
            event: "message".to_owned(),
            data: "[DONE]".to_owned(),
        }]
    );
}

#[test]
fn parser_rejects_invalid_utf8_robust() {
    let mut parser = SseParser::new();
    let err = parser.push(b"data: \xFF\n\n").unwrap_err();
    assert!(err.to_string().contains("valid UTF-8"));
}

#[test]
fn parser_rejects_unbounded_line_robust() {
    let mut parser = SseParser::new();
    let line = vec![b'a'; TEST_MAX_LINE_BYTES + 1];
    let err = parser.push(&line).unwrap_err();
    assert!(err.to_string().contains("exceeded"));
}

#[test]
fn byte_stream_timeout_parser_and_clamp_normal() {
    assert_eq!(stream::parse_timeout_ms(None), None);
    assert_eq!(stream::parse_timeout_ms(Some("0")), None);
    assert_eq!(
        stream::parse_timeout_ms(Some("15000")),
        Some(Duration::from_secs(15))
    );
    assert_eq!(
        stream::clamp_byte_stream_timeout(Duration::from_millis(1)),
        stream::MIN_BYTE_STREAM_IDLE_TIMEOUT
    );
    assert_eq!(
        stream::clamp_byte_stream_timeout(Duration::from_secs(3600)),
        stream::MAX_BYTE_STREAM_IDLE_TIMEOUT
    );
}

#[tokio::test]
async fn byte_stream_events_preserves_frame_order_normal() {
    let chunks = futures::stream::iter([
        Ok::<_, std::io::Error>(b"data: one\n\n".to_vec()),
        Ok::<_, std::io::Error>(b"event: two\ndata: 2\n\n".to_vec()),
    ]);
    let frames = byte_stream_events(chunks)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(frames[0].data, "one");
    assert_eq!(frames[1].event, "two");
}

#[test]
fn parser_trace_detail_records_addresses_normal() {
    linkscope::trace_detail_enable();

    let mut parser = SseParser::new();
    let frames = parser.push(b"event: ping\ndata: {}\n\n").unwrap();
    assert_eq!(frames.len(), 1);

    let snapshot = linkscope::snapshot();
    assert!(
        snapshot
            .traces
            .iter()
            .any(|trace| trace.label == "sdk.sse.parser.push.input"
                && trace.fields.iter().any(|field| field.name == "input_addr"))
    );
    assert!(
        snapshot
            .traces
            .iter()
            .any(|trace| trace.label == "sdk.sse.parser.dispatch.frame"
                && trace.fields.iter().any(|field| field.name == "data_addr"))
    );
}
