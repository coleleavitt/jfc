use jfc_session::{
    BranchForkSummary, CompactionBoundary, ContextEvent, CustomPluginEntry, LabelEntry,
    MessageContentPart, MessageMetadata, ModelChange, SessionEntry, SessionEntryId,
    SessionEntryKind, ThinkingChange, ToolResult, ToolUse,
};
use serde_json::json;

fn entry(id: &str, parent_id: Option<&str>, kind: SessionEntryKind) -> SessionEntry {
    SessionEntry {
        id: SessionEntryId::from(id),
        parent_id: parent_id.map(SessionEntryId::from),
        timestamp: "2026-06-27T00:00:00Z".to_owned(),
        kind,
    }
}

#[test]
fn session_entry_round_trips_representative_entries_normal() {
    let entries = vec![
        entry(
            "e1",
            None,
            SessionEntryKind::UserMessage {
                content: vec![MessageContentPart::Text {
                    content: "hello".to_owned(),
                }],
                metadata: MessageMetadata::default(),
            },
        ),
        entry(
            "e2",
            Some("e1"),
            SessionEntryKind::AssistantMessage {
                content: vec![
                    MessageContentPart::Thinking {
                        content: "plan".to_owned(),
                    },
                    MessageContentPart::ThinkingSignature {
                        signature: "sig".to_owned(),
                    },
                    MessageContentPart::Text {
                        content: "answer".to_owned(),
                    },
                    MessageContentPart::RedactedThinking {
                        data: "opaque".to_owned(),
                    },
                ],
                metadata: MessageMetadata {
                    model_name: Some("claude-sonnet".to_owned()),
                    usage: Some(json!({ "input_tokens": 10, "output_tokens": 3 })),
                    ..MessageMetadata::default()
                },
            },
        ),
        entry(
            "e3",
            Some("e2"),
            SessionEntryKind::ToolUse(ToolUse {
                tool_use_id: "toolu_1".to_owned(),
                kind: "bash".to_owned(),
                input: json!({ "command": "cargo test -p jfc-session" }),
                thought_signature: Some("provider-sig".to_owned()),
            }),
        ),
        entry(
            "e4",
            Some("e3"),
            SessionEntryKind::ToolResult(ToolResult {
                tool_use_id: "toolu_1".to_owned(),
                status: "complete".to_owned(),
                output: json!({ "stdout": "ok" }),
            }),
        ),
        entry(
            "e5",
            Some("e4"),
            SessionEntryKind::ModelChange(ModelChange {
                provider: Some("anthropic".to_owned()),
                model_id: "claude-sonnet".to_owned(),
            }),
        ),
        entry(
            "e6",
            Some("e5"),
            SessionEntryKind::ThinkingChange(ThinkingChange {
                level: "medium".to_owned(),
            }),
        ),
        entry(
            "e7",
            Some("e6"),
            SessionEntryKind::CompactionBoundary(CompactionBoundary {
                summary: "previous work".to_owned(),
                first_kept_entry_id: Some(SessionEntryId::from("e5")),
                tokens_before: Some(4096),
            }),
        ),
        entry(
            "e8",
            Some("e7"),
            SessionEntryKind::BranchForkSummary(BranchForkSummary {
                from_id: SessionEntryId::from("e4"),
                summary: "forked for experiment".to_owned(),
                details: json!({ "branch": "try-dto" }),
            }),
        ),
        entry(
            "e9",
            Some("e8"),
            SessionEntryKind::CustomPluginEntry(CustomPluginEntry {
                plugin_id: "plugin.example".to_owned(),
                custom_type: "plugin_state".to_owned(),
                data: json!({ "unknown_future_key": true }),
            }),
        ),
        entry(
            "e10",
            Some("e9"),
            SessionEntryKind::Label(LabelEntry {
                target_id: SessionEntryId::from("e2"),
                label: Some("important answer".to_owned()),
            }),
        ),
        entry(
            "e11",
            Some("e10"),
            SessionEntryKind::ContextEvent(ContextEvent {
                name: "cwd".to_owned(),
                data: json!({ "path": "/tmp/project" }),
            }),
        ),
    ];

    let json = serde_json::to_string_pretty(&entries).unwrap();
    println!("representative session entry JSON:\n{json}");

    assert!(json.contains("\"type\": \"user_message\""));
    assert!(json.contains("\"type\": \"custom_plugin_entry\""));
    let back: Vec<SessionEntry> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, entries);
}

#[test]
fn session_entry_custom_plugin_round_trips_unknown_data_robust() {
    let raw = json!({
        "id": "future-1",
        "parent_id": null,
        "timestamp": "2026-06-27T00:00:00Z",
        "type": "custom_plugin_entry",
        "plugin_id": "future.plugin",
        "custom_type": "new_kind_not_known_to_jfc",
        "data": {
            "nested": { "x": 1 },
            "array": [true, "kept"]
        }
    });

    let parsed: SessionEntry = serde_json::from_value(raw.clone()).unwrap();
    let encoded = serde_json::to_value(parsed).unwrap();

    assert_eq!(encoded, raw);
}

#[test]
fn session_entry_malformed_custom_plugin_errors_robust() {
    let missing_custom_type = json!({
        "id": "bad-1",
        "parent_id": null,
        "timestamp": "2026-06-27T00:00:00Z",
        "type": "custom_plugin_entry",
        "plugin_id": "plugin.example",
        "data": {}
    });
    let empty_plugin_id = json!({
        "id": "bad-2",
        "parent_id": null,
        "timestamp": "2026-06-27T00:00:00Z",
        "type": "custom_plugin_entry",
        "plugin_id": " ",
        "custom_type": "state",
        "data": {}
    });
    let unknown_top_level_type = json!({
        "id": "bad-3",
        "parent_id": null,
        "timestamp": "2026-06-27T00:00:00Z",
        "type": "surprise_runtime_entry",
        "data": {}
    });

    assert!(serde_json::from_value::<SessionEntry>(missing_custom_type).is_err());
    assert!(serde_json::from_value::<SessionEntry>(empty_plugin_id).is_err());
    assert!(serde_json::from_value::<SessionEntry>(unknown_top_level_type).is_err());
}
