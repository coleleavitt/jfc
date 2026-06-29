use super::*;

#[test]
fn protocol_version_exists() {
    assert_eq!(PROTOCOL_VERSION, 1);
}

#[test]
fn envelope_roundtrip_assistant_delta() {
    let env = RemoteEnvelope::AssistantDelta {
        text: Some("Hello".into()),
        reasoning: None,
    };
    let json = serde_json::to_string(&env).unwrap();
    let back: RemoteEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(env, back);
}

#[test]
fn envelope_roundtrip_user_prompt() {
    let env = RemoteEnvelope::UserPrompt {
        text: "fix the bug".into(),
    };
    let json = serde_json::to_string(&env).unwrap();
    assert!(json.contains("\"type\":\"user_prompt\""));
    let back: RemoteEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(env, back);
}

#[test]
fn envelope_roundtrip_permission_request() {
    let env = RemoteEnvelope::PermissionRequest {
        tool_use_id: "t1".into(),
        tool_name: "Bash".into(),
        summary: "rm -rf /".into(),
        diff: Some("- old\n+ new".into()),
    };
    let json = serde_json::to_string(&env).unwrap();
    let back: RemoteEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(env, back);
}

#[test]
fn tool_use_summary_timestamp_is_optional_robust() {
    let json =
        r#"{"type":"tool_use_summary","summary":"edited files","preceding_tool_use_ids":["t1"]}"#;
    let env: RemoteEnvelope = serde_json::from_str(json).unwrap();
    assert_eq!(
        env,
        RemoteEnvelope::ToolUseSummary {
            summary: "edited files".into(),
            preceding_tool_use_ids: vec!["t1".into()],
            timestamp: None,
        }
    );
    assert!(env.is_outbound());
}

#[test]
fn frame_roundtrip() {
    let frame = RemoteFrame {
        version: PROTOCOL_VERSION,
        seq: 42,
        ts_ms: 1700000000000,
        payload: RemoteEnvelope::Heartbeat,
        hmac: "abc123".into(),
    };
    let json = serde_json::to_string(&frame).unwrap();
    let back: RemoteFrame = serde_json::from_str(&json).unwrap();
    assert_eq!(frame, back);
}

#[test]
fn direction_helpers() {
    assert!(RemoteEnvelope::Heartbeat.is_outbound());
    assert!(!RemoteEnvelope::Heartbeat.is_inbound());
    assert!(RemoteEnvelope::Ping.is_inbound());
    assert!(!RemoteEnvelope::Ping.is_outbound());
}

#[test]
fn envelope_direction_trace_records_kind_without_payload_normal() {
    linkscope::trace_detail_enable();
    let env = RemoteEnvelope::UserPrompt {
        text: "sensitive prompt text".into(),
    };
    assert!(env.is_inbound());

    let snapshot = linkscope::snapshot();
    let rendered = format!("{snapshot:?}");
    assert!(rendered.contains("remote.protocol.envelope.direction"));
    assert!(rendered.contains("user_prompt"));
    assert!(!rendered.contains("sensitive prompt text"));
}

#[test]
fn session_state_serialization() {
    let s = SessionState::WaitingApproval;
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(json, "\"waiting_approval\"");
}
