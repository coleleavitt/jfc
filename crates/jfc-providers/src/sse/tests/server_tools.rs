use super::*;

#[test]
fn server_tool_use_content_block_parses() {
    let json = r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtool_1","name":"web_search","input":{"query":"rust async"}}}"#;
    let event: SseEvent = serde_json::from_str(json).expect("server_tool_use must parse");
    assert!(matches!(
        event,
        SseEvent::ContentBlockStart {
            content_block: ContentBlock::ServerToolUse { .. },
            ..
        }
    ));
}

#[test]
fn server_tool_use_block_emits_tool_done_with_prefix() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::ServerToolUse {
                id: "srvtool_1".into(),
                name: "web_search".into(),
                input: serde_json::json!({"query": "rust async"}),
            },
        },
        &mut blocks,
        &mut sr,
    );
    assert!(matches!(blocks[0], Some(BlockState::ServerToolUse { .. })));

    let out = translate(
        SseEvent::ContentBlockStop { index: 0 },
        &mut blocks,
        &mut sr,
    );
    // ToolDone is emitted with "server_tool_use:" prefix so stream.rs
    // can route to a non-dispatch path.
    assert!(
        matches!(out, Some(StreamEvent::ToolDone { ref tool_name, ref tool_use_id, .. })
                if tool_name == "server_tool_use:web_search" && tool_use_id == "srvtool_1"),
        "expected ToolDone with server_tool_use: prefix, got: {out:?}"
    );
    assert!(blocks[0].is_none());
}

// Regression: a parse error mid-stream must flush an open text block as a
// TextDone so accumulated text isn't dropped before the error surfaces.

#[test]
fn server_tool_use_streamed_input_json_accumulates() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::ServerToolUse {
                id: "srvtool_1".into(),
                name: "web_search".into(),
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
                partial_json: r#"{"query":"weat"#.into(),
            },
        },
        &mut blocks,
        &mut sr,
    );
    translate(
        SseEvent::ContentBlockDelta {
            index: 0,
            delta: Delta::InputJsonDelta {
                partial_json: r#"her"}"#.into(),
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
        matches!(out, Some(StreamEvent::ToolDone { ref input_json, .. })
                if input_json == r#"{"query":"weather"}"#),
        "expected accumulated server tool input, got: {out:?}"
    );
}

#[test]
fn server_tool_use_null_input_produces_empty_string() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::ServerToolUse {
                id: "srvtool_2".into(),
                name: "code_execution".into(),
                input: serde_json::Value::Null,
            },
        },
        &mut blocks,
        &mut sr,
    );
    if let Some(Some(BlockState::ServerToolUse { input, .. })) = blocks.first() {
        assert!(input.is_empty(), "null input should become empty string");
    } else {
        panic!("expected ServerToolUse block state");
    }
}

#[test]
fn server_tool_use_from_name_routes_to_server_variant() {
    use jfc_core::ToolKind;
    assert!(
        matches!(
            ToolKind::from_name("server_tool_use:web_search"),
            ToolKind::ServerWebSearch
        ),
        "server_tool_use:web_search should map to ServerWebSearch"
    );
    assert!(
        matches!(
            ToolKind::from_name("server_tool_use:code_execution"),
            ToolKind::ServerCodeExecution
        ),
        "server_tool_use:code_execution should map to ServerCodeExecution"
    );
    assert!(
        matches!(
            ToolKind::from_name("server_tool_use:advisor"),
            ToolKind::ServerAdvisor
        ),
        "server_tool_use:advisor should map to ServerAdvisor"
    );
    assert!(
        matches!(
            ToolKind::from_name("server_tool_use:unknown_future_tool"),
            ToolKind::Generic(_)
        ),
        "unknown server tool should fall through to Generic"
    );
}

#[test]
fn advisor_tool_result_block_emits_server_result() {
    let (mut blocks, mut sr) = empty_state();
    translate(
        SseEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock::ServerToolResult {
                tool_use_id: "srvtool_advisor".into(),
                tool_kind: ServerToolResultKind::Advisor,
                content: serde_json::json!({"type":"advisor_result","text":"check edge cases"}),
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
        matches!(out, Some(StreamEvent::ServerToolResult { ref tool_use_id, ref tool_kind, .. })
                if tool_use_id == "srvtool_advisor"
                    && *tool_kind == ServerToolResultKind::Advisor),
        "expected advisor ServerToolResult, got: {out:?}"
    );
}
