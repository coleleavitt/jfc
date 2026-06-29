use super::*;

#[test]
fn translate_tool_use_lifecycle() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "bash".into(),
                input: Value::Null,
            },
        },
        &mut blocks,
        &mut sr,
    );
    translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::InputJsonDelta {
                partial_json: r#"{"cmd":"ls"}"#.into(),
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
    assert!(
        matches!(out, Some(StreamEvent::ToolDone { tool_name, tool_use_id, input_json, .. })
            if tool_name == "bash" && tool_use_id == "tu_1" && input_json == r#"{"cmd":"ls"}"#)
    );
}
