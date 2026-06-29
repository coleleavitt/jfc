use jfc_core::{SessionId, TaskInput, ToolId};
use jfc_plugin_sdk::{
    BridgeAgentLaunchRequest, BridgeAgentLaunchResult, BridgeEnvelope, BridgeErrorDto,
    BridgePromptContextRefreshRequest, BridgePromptContextRefreshResult, BridgeProviderContent,
    BridgeProviderMessage, BridgeProviderRole, BridgeProviderStreamEvent,
    BridgeProviderStreamOptions, BridgeRequest, BridgeResponse, BridgeStopReason,
    BridgeUiPanelRefreshRequest, BridgeUiPanelRefreshResult, BridgeUiWidgetRefreshRequest,
    BridgeUiWidgetRefreshResult, HookName, UiMutationScope,
};

#[test]
fn bridge_frames_reuse_core_ids_and_round_trip_hook_requests() {
    // Given: a process-bridge hook request using stable jfc-core ids.
    let request = BridgeEnvelope::request(
        "req-1",
        BridgeRequest::Hook {
            hook: HookName::PreToolUse,
            session_id: SessionId::new("ses_1"),
            tool_id: Some(ToolId::new("tool_1")),
            payload: serde_json::json!({"tool":"bash"}),
        },
    );

    // When: the frame is serialized and deserialized over JSONL.
    let json = serde_json::to_string(&request).expect("bridge frame serializes");
    let round_trip: BridgeEnvelope =
        serde_json::from_str(&json).expect("bridge frame deserializes");

    // Then: typed ids and hook names are preserved.
    assert_eq!(round_trip.id(), "req-1");
    assert!(json.contains("pre_tool_use"));
    assert!(json.contains("ses_1"));
}

#[test]
fn bridge_error_response_serializes_structured_compatibility_error() {
    // Given: an error response emitted by a plugin bridge.
    let response = BridgeEnvelope::response(
        "req-2",
        BridgeResponse::Error(BridgeErrorDto::new(
            "invalid_manifest",
            "manifest schema is invalid",
        )),
    );

    // When: the response crosses the JSON boundary.
    let json = serde_json::to_string(&response).expect("bridge error serializes");

    // Then: callers can inspect the stable error code without parsing Display text.
    assert!(json.contains("invalid_manifest"));
    assert!(json.contains("manifest schema is invalid"));
}

#[test]
fn bridge_tool_call_and_result_round_trip_over_jsonl() {
    // Given: a plugin-owned tool call crossing a process bridge.
    let request = BridgeEnvelope::request(
        "tool-1",
        BridgeRequest::ToolCall {
            tool: "external_echo".to_owned(),
            tool_id: Some(ToolId::new("tool_2")),
            input: serde_json::json!({"message":"hi"}),
        },
    );
    let response = BridgeEnvelope::response(
        "tool-1",
        BridgeResponse::ToolResult {
            output: "bridge ok".to_owned(),
            is_error: false,
            payload: None,
        },
    );

    // When: both sides serialize frames as JSONL payloads.
    let request_json = serde_json::to_string(&request).expect("tool request serializes");
    let response_json = serde_json::to_string(&response).expect("tool response serializes");
    let request_round_trip: BridgeEnvelope =
        serde_json::from_str(&request_json).expect("tool request deserializes");
    let response_round_trip: BridgeEnvelope =
        serde_json::from_str(&response_json).expect("tool response deserializes");

    // Then: the tool contract keeps a typed tool name, id, input, and result.
    assert_eq!(request_round_trip.id(), "tool-1");
    assert_eq!(response_round_trip.id(), "tool-1");
    assert!(request_json.contains("external_echo"));
    assert!(request_json.contains("tool_call"));
    assert!(response_json.contains("tool_result"));
    assert!(response_json.contains("bridge ok"));
}

#[test]
fn bridge_provider_stream_request_and_events_round_trip_over_jsonl() {
    // Given: a plugin-owned provider stream request and the event frames it returns.
    let request = BridgeEnvelope::request(
        "provider-1",
        BridgeRequest::ProviderStream {
            provider: "local-ai".to_owned(),
            messages: vec![BridgeProviderMessage {
                role: BridgeProviderRole::User,
                content: vec![BridgeProviderContent::Text {
                    text: "hello".to_owned(),
                }],
            }],
            options: BridgeProviderStreamOptions::new("local-chat").max_tokens(128),
        },
    );
    let event = BridgeEnvelope::response(
        "provider-1",
        BridgeResponse::ProviderEvent {
            event: BridgeProviderStreamEvent::TextDelta {
                index: 0,
                delta: "hi".to_owned(),
            },
        },
    );
    let done = BridgeEnvelope::response(
        "provider-1",
        BridgeResponse::ProviderEvent {
            event: BridgeProviderStreamEvent::Done {
                stop_reason: BridgeStopReason::EndTurn,
            },
        },
    );

    // When: the frames cross the JSONL boundary.
    let request_json = serde_json::to_string(&request).expect("provider request serializes");
    let event_json = serde_json::to_string(&event).expect("provider event serializes");
    let done_json = serde_json::to_string(&done).expect("provider done serializes");
    let request_round_trip: BridgeEnvelope =
        serde_json::from_str(&request_json).expect("provider request deserializes");
    let event_round_trip: BridgeEnvelope =
        serde_json::from_str(&event_json).expect("provider event deserializes");
    let done_round_trip: BridgeEnvelope =
        serde_json::from_str(&done_json).expect("provider done deserializes");

    // Then: provider identity, model options, deltas, and finish reason survive.
    assert_eq!(request_round_trip.id(), "provider-1");
    assert_eq!(event_round_trip.id(), "provider-1");
    assert_eq!(done_round_trip.id(), "provider-1");
    assert!(request_json.contains("provider_stream"));
    assert!(request_json.contains("local-ai"));
    assert!(request_json.contains("local-chat"));
    assert!(event_json.contains("text_delta"));
    assert!(done_json.contains("end_turn"));
}

#[test]
fn bridge_agent_launch_request_and_result_round_trip_over_jsonl() {
    // Given: a plugin-owned agent launch request and completion response.
    let task = TaskInput {
        description: "inspect code".to_owned(),
        prompt: "find the sharp edges".to_owned(),
        subagent_type: Some("reviewer".to_owned()),
        category: None,
        run_in_background: false,
        model: Some("local-model".to_owned()),
        launcher: None,
        effort: Some("high".to_owned()),
        name: None,
        team_name: None,
        mode: None,
        isolation: None,
        parent_task_id: None,
        schema: None,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        cwd: Some("/workspace/project".to_owned()),
    };
    let request = BridgeEnvelope::request(
        "agent-1",
        BridgeRequest::AgentLaunch {
            launch: BridgeAgentLaunchRequest::new("variant-agent", task)
                .with_task_id("task_1")
                .with_model("local-model")
                .with_provider("plugin-provider")
                .with_cwd("/workspace/project"),
        },
    );
    let response = BridgeEnvelope::response(
        "agent-1",
        BridgeResponse::AgentLaunchResult {
            result: BridgeAgentLaunchResult::success("agent finished"),
        },
    );

    // When: the frames cross the JSONL boundary.
    let request_json = serde_json::to_string(&request).expect("agent launch request serializes");
    let response_json = serde_json::to_string(&response).expect("agent launch result serializes");
    let request_round_trip: BridgeEnvelope =
        serde_json::from_str(&request_json).expect("agent launch request deserializes");
    let response_round_trip: BridgeEnvelope =
        serde_json::from_str(&response_json).expect("agent launch result deserializes");

    // Then: launcher identity, task payload, runtime hints, and result are preserved.
    assert_eq!(request_round_trip.id(), "agent-1");
    assert_eq!(response_round_trip.id(), "agent-1");
    assert!(request_json.contains("agent_launch"));
    assert!(request_json.contains("variant-agent"));
    assert!(request_json.contains("local-model"));
    assert!(response_json.contains("agent_launch_result"));
    assert!(response_json.contains("agent finished"));
}

#[test]
fn bridge_ui_widget_refresh_request_and_result_round_trip_over_jsonl() {
    let request = BridgeEnvelope::request(
        "widget-1",
        BridgeRequest::UiWidgetRefresh {
            refresh: BridgeUiWidgetRefreshRequest::new(
                "review.queue",
                UiMutationScope::InfoSidebar,
            )
            .with_state(serde_json::json!({ "cursor": "abc" })),
        },
    );
    let response = BridgeEnvelope::response(
        "widget-1",
        BridgeResponse::UiWidgetRefresh {
            result: BridgeUiWidgetRefreshResult::body("4 open reviews")
                .with_state(serde_json::json!({ "cursor": "def" })),
        },
    );

    let request_json = serde_json::to_string(&request).expect("widget request serializes");
    let response_json = serde_json::to_string(&response).expect("widget response serializes");
    let request_round_trip: BridgeEnvelope =
        serde_json::from_str(&request_json).expect("widget request deserializes");
    let response_round_trip: BridgeEnvelope =
        serde_json::from_str(&response_json).expect("widget response deserializes");

    assert_eq!(request_round_trip.id(), "widget-1");
    assert_eq!(response_round_trip.id(), "widget-1");
    assert!(request_json.contains("ui_widget_refresh"));
    assert!(request_json.contains("review.queue"));
    assert!(request_json.contains("info_sidebar"));
    assert!(response_json.contains("4 open reviews"));
}

#[test]
fn bridge_ui_panel_refresh_request_and_result_round_trip_over_jsonl() {
    let request = BridgeEnvelope::request(
        "panel-1",
        BridgeRequest::UiPanelRefresh {
            refresh: BridgeUiPanelRefreshRequest::new(
                "review.summary",
                UiMutationScope::InfoSidebar,
            )
            .with_state(serde_json::json!({ "cursor": "abc" })),
        },
    );
    let response = BridgeEnvelope::response(
        "panel-1",
        BridgeResponse::UiPanelRefresh {
            result: BridgeUiPanelRefreshResult::body("4 open reviews\n1 blocking approval")
                .with_state(serde_json::json!({ "cursor": "def" })),
        },
    );

    let request_json = serde_json::to_string(&request).expect("panel request serializes");
    let response_json = serde_json::to_string(&response).expect("panel response serializes");
    let request_round_trip: BridgeEnvelope =
        serde_json::from_str(&request_json).expect("panel request deserializes");
    let response_round_trip: BridgeEnvelope =
        serde_json::from_str(&response_json).expect("panel response deserializes");

    assert_eq!(request_round_trip.id(), "panel-1");
    assert_eq!(response_round_trip.id(), "panel-1");
    assert!(request_json.contains("ui_panel_refresh"));
    assert!(request_json.contains("review.summary"));
    assert!(request_json.contains("info_sidebar"));
    assert!(response_json.contains("4 open reviews"));
}

#[test]
fn bridge_prompt_context_refresh_request_and_result_round_trip_over_jsonl() {
    let request = BridgeEnvelope::request(
        "context-1",
        BridgeRequest::PromptContextRefresh {
            refresh: BridgePromptContextRefreshRequest::new("context.repo")
                .with_cwd("/workspace/repo")
                .with_max_chars(4096)
                .with_state(serde_json::json!({ "etag": "abc" })),
        },
    );
    let response = BridgeEnvelope::response(
        "context-1",
        BridgeResponse::PromptContextRefresh {
            result: BridgePromptContextRefreshResult::body("Repository rules")
                .with_state(serde_json::json!({ "etag": "def" })),
        },
    );

    let request_json = serde_json::to_string(&request).expect("context request serializes");
    let response_json = serde_json::to_string(&response).expect("context response serializes");
    let request_round_trip: BridgeEnvelope =
        serde_json::from_str(&request_json).expect("context request deserializes");
    let response_round_trip: BridgeEnvelope =
        serde_json::from_str(&response_json).expect("context response deserializes");

    assert_eq!(request_round_trip.id(), "context-1");
    assert_eq!(response_round_trip.id(), "context-1");
    assert!(request_json.contains("prompt_context_refresh"));
    assert!(request_json.contains("context.repo"));
    assert!(request_json.contains("/workspace/repo"));
    assert!(response_json.contains("Repository rules"));
}
