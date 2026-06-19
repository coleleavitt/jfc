use super::*;

#[test]
fn citations_delta_parses_and_ignored() {
    let json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta"}}"#;
    let event: SseEvent = serde_json::from_str(json).expect("citations_delta must parse");
    let (mut blocks, mut sr) = empty_state();
    blocks.push(Some(BlockState::Text {
        accumulated: String::new(),
    }));
    assert!(translate(event, &mut blocks, &mut sr).is_none());
}

#[test]
fn connector_text_delta_parses_and_ignored() {
    let json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"connector_text_delta","connector_text":"\n\n"}}"#;
    let event: SseEvent = serde_json::from_str(json).expect("connector_text_delta must parse");
    let (mut blocks, mut sr) = empty_state();
    blocks.push(Some(BlockState::Text {
        accumulated: String::new(),
    }));
    assert!(translate(event, &mut blocks, &mut sr).is_none());
}

#[test]
fn unknown_delta_type_parses_and_ignored() {
    let json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"totally_new_delta","data":"x"}}"#;
    let event: SseEvent = serde_json::from_str(json).expect("unknown delta should parse");
    let (mut blocks, mut sr) = empty_state();
    assert!(translate(event, &mut blocks, &mut sr).is_none());
}
