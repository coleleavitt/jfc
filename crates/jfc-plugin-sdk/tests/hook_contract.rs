use std::str::FromStr;

use jfc_plugin_sdk::{HookDescriptor, HookName, PluginId};

#[test]
fn hook_name_rejects_unknown_string_when_parsing_rust_api_input() {
    // Given: an untrusted hook name string from a plugin boundary.
    let raw_name = "definitely_not_a_hook";

    // When: Rust code parses it into the typed HookName API.
    let result = HookName::from_str(raw_name);

    // Then: the string is rejected instead of becoming an open-ended hook.
    assert!(result.is_err());
}

#[test]
fn hook_name_rejects_unknown_string_when_deserializing() {
    // Given: a descriptor payload with an unknown hook name.
    let payload = r#"{"plugin_id":"example","name":"definitely_not_a_hook"}"#;

    // When: the descriptor crosses the serde boundary.
    let result = serde_json::from_str::<HookDescriptor>(payload);

    // Then: serde rejects the unknown hook name.
    assert!(result.is_err());
}

#[test]
fn hook_descriptor_requires_typed_hook_name_in_rust_api() {
    // Given: a typed plugin id and typed hook name.
    let descriptor = HookDescriptor::new(PluginId::new("example"), HookName::PreToolUse);

    // When: the descriptor is serialized for the bridge or host registry.
    let json = serde_json::to_string(&descriptor).expect("hook descriptor serializes");

    // Then: the stable hook string is emitted, and no raw free-form string constructor is needed.
    assert!(json.contains("pre_tool_use"));
    assert_eq!(descriptor.name(), HookName::PreToolUse);
}
