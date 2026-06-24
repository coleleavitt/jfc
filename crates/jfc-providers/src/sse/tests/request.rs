use super::*;

#[test]
fn build_messages_roundtrip() {
    let msgs = vec![
        make_user_msg("q1"),
        make_assistant_msg("a1"),
        make_user_msg("q2"),
    ];
    let v = build_messages(&msgs);
    assert_eq!(v[0]["role"], "user");
    assert_eq!(v[0]["content"][0]["text"], "q1");
    assert_eq!(v[1]["role"], "assistant");
    assert_eq!(v[2]["role"], "user");
}

#[test]
fn build_messages_empty() {
    let v = build_messages(&[]);
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[test]
fn build_messages_tool_result_shape() {
    let msg = ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::ToolResult {
            tool_use_id: "tu_1".into(),
            content: "output".into(),
            is_error: false,
        }],
    };
    let v = build_messages(&[msg]);
    let block = &v[0]["content"][0];
    assert_eq!(block["type"], "tool_result");
    assert_eq!(block["tool_use_id"], "tu_1");
    assert_eq!(block["is_error"], false);
}

#[test]
fn build_messages_tool_use_shape() {
    let msg = ProviderMessage {
        role: ProviderRole::Assistant,
        content: vec![ProviderContent::ToolUse {
            id: "tu_2".into(),
            name: "read_file".into(),
            input: serde_json::json!({"path": "/tmp/x"}),
            thought_signature: None,
        }],
    };
    let v = build_messages(&[msg]);
    let block = &v[0]["content"][0];
    assert_eq!(block["type"], "tool_use");
    assert_eq!(block["id"], "tu_2");
    assert_eq!(block["name"], "read_file");
}

#[test]
fn build_messages_thinking_shape_includes_signature_regression() {
    let msg = ProviderMessage {
        role: ProviderRole::Assistant,
        content: vec![ProviderContent::Thinking {
            text: "thinking".into(),
            signature: Some("sig_1".into()),
        }],
    };
    let v = build_messages(&[msg]);
    let block = &v[0]["content"][0];
    assert_eq!(block["type"], "thinking");
    assert_eq!(block["thinking"], "thinking");
    assert_eq!(block["signature"], "sig_1");
}

#[test]
fn build_messages_omits_blank_text_blocks_robust() {
    let msg = ProviderMessage {
        role: ProviderRole::User,
        content: vec![
            ProviderContent::Text(String::new()),
            ProviderContent::Text("  ".into()),
            ProviderContent::Text("hello".into()),
        ],
    };
    let v = build_messages(&[msg]);
    let content = v[0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["text"], "hello");
}

#[test]
fn build_messages_blank_only_message_uses_placeholder_robust() {
    let msg = ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(String::new())],
    };
    let v = build_messages(&[msg]);
    let content = v[0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], ".");
    assert!(!content[0]["text"].as_str().unwrap().trim().is_empty());
}

#[test]
fn build_tools_shape() {
    let tools = vec![ToolDef {
        name: "bash".into(),
        description: "Execute bash".into(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let v = build_tools(&tools);
    let arr = v.as_array().unwrap();
    assert_eq!(arr[0]["name"], "bash");
}

#[test]
fn apply_anthropic_tool_schema_controls_skips_server_tools() {
    let tools = vec![ToolDef {
        name: "bash".into(),
        description: "Execute bash".into(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let model = ModelId::from("claude-opus-4-7");
    let mut v = build_tools_with_advisor(&tools, Some(&model));
    apply_anthropic_tool_schema_controls(&mut v, true, true);
    let arr = v.as_array().unwrap();
    assert_eq!(arr[0]["eager_input_streaming"], true);
    assert_eq!(arr[0]["strict"], true);
    assert!(arr[1].get("eager_input_streaming").is_none());
    assert!(arr[1].get("strict").is_none());
}

#[test]
fn build_tools_empty() {
    let v = build_tools(&[]);
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[test]
fn build_tools_order_preserved() {
    let tools: Vec<ToolDef> = ["alpha", "beta", "gamma"]
        .iter()
        .map(|n| ToolDef {
            name: n.to_string(),
            description: n.to_string(),
            input_schema: serde_json::json!({}),
        })
        .collect();
    let v = build_tools(&tools);
    let arr = v.as_array().unwrap();
    assert_eq!(arr[0]["name"], "alpha");
    assert_eq!(arr[1]["name"], "beta");
    assert_eq!(arr[2]["name"], "gamma");
}

#[test]
fn ensure_input_object_passes_objects_through() {
    let obj = serde_json::json!({"path": "/tmp", "recursive": true});
    let result = ensure_input_object(&obj);
    assert_eq!(result, obj);
}

#[test]
fn ensure_input_object_parses_stringified_json() {
    let s = serde_json::Value::String(r#"{"path":"/tmp"}"#.to_owned());
    let result = ensure_input_object(&s);
    assert_eq!(result, serde_json::json!({"path": "/tmp"}));
}

#[test]
fn ensure_input_object_empty_string_becomes_empty_object() {
    let s = serde_json::Value::String("".to_owned());
    assert_eq!(ensure_input_object(&s), serde_json::json!({}));
}

#[test]
fn ensure_input_object_null_string_becomes_empty_object() {
    let s = serde_json::Value::String("null".to_owned());
    assert_eq!(ensure_input_object(&s), serde_json::json!({}));
}

#[test]
fn ensure_input_object_null_value_becomes_empty_object() {
    assert_eq!(
        ensure_input_object(&serde_json::Value::Null),
        serde_json::json!({})
    );
}

#[test]
fn ensure_input_object_unparseable_string_becomes_empty_object() {
    let s = serde_json::Value::String("not json at all".to_owned());
    assert_eq!(ensure_input_object(&s), serde_json::json!({}));
}

#[test]
fn ensure_input_object_string_array_gets_wrapped() {
    let s = serde_json::Value::String("[1, 2, 3]".to_owned());
    let result = ensure_input_object(&s);
    assert_eq!(result, serde_json::json!({"value": [1, 2, 3]}));
}

// ─── server_tool_use tests ────────────────────────────────────────────────
