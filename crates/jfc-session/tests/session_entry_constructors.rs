use jfc_session::{
    CustomPluginEntry, MessageContentPart, MessageMetadata, SessionEntry, SessionEntryId,
    SessionEntryKind, SessionEntryValidationError,
};
use serde_json::json;

#[test]
fn session_entry_typed_constructors_validate_ids_normal() {
    let entry = SessionEntry::new(
        SessionEntryId::parse("entry-1").unwrap(),
        "2026-06-27T00:00:00Z",
        SessionEntryKind::user_message(vec![MessageContentPart::text("hello")]),
    )
    .with_parent(SessionEntryId::parse("parent-1").unwrap());

    assert_eq!(entry.id.as_str(), "entry-1");
    assert_eq!(
        entry.parent_id.as_ref().map(SessionEntryId::as_str),
        Some("parent-1")
    );
    assert_eq!(entry.timestamp, "2026-06-27T00:00:00Z");
    assert!(entry.validate().is_ok());
}

#[test]
fn session_entry_validation_rejects_malformed_values_robust() {
    assert_eq!(
        SessionEntryId::parse(" ").unwrap_err(),
        SessionEntryValidationError::EmptySessionEntryId,
    );

    let malformed = SessionEntry {
        id: SessionEntryId::new("ok"),
        parent_id: Some(SessionEntryId::new("")),
        timestamp: " ".to_owned(),
        kind: SessionEntryKind::assistant_message_with_metadata(
            vec![MessageContentPart::text("answer")],
            MessageMetadata::default(),
        ),
    };

    assert_eq!(
        malformed.validate().unwrap_err(),
        SessionEntryValidationError::EmptyParentSessionEntryId,
    );
}

#[test]
fn session_entry_custom_plugin_constructor_rejects_malformed_identifiers_robust() {
    let plugin = CustomPluginEntry::new(
        "plugin.example",
        "state_snapshot",
        json!({ "unknown_future_key": true }),
    )
    .unwrap();

    assert_eq!(plugin.plugin_id, "plugin.example");
    assert_eq!(plugin.custom_type, "state_snapshot");
    assert_eq!(plugin.data, json!({ "unknown_future_key": true }));
    assert_eq!(
        CustomPluginEntry::new("", "state_snapshot", json!({})).unwrap_err(),
        SessionEntryValidationError::EmptyPluginId,
    );
    assert_eq!(
        CustomPluginEntry::new("plugin.example", " ", json!({})).unwrap_err(),
        SessionEntryValidationError::EmptyCustomType,
    );
}
