use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeMailboxMessage, BridgeMailboxPollRequest, BridgeMailboxSendRequest,
    BridgeRequest, BridgeResponse, BridgeTeammateEvent, BridgeTeammateReady,
};

#[test]
fn bridge_teammate_events_round_trip_over_jsonl() {
    let frames = teammate_event_frames()
        .into_iter()
        .map(|frame| serde_json::to_string(&frame).expect("teammate event serializes"))
        .collect::<Vec<_>>();
    let round_trips = frames
        .iter()
        .map(|json| serde_json::from_str::<BridgeEnvelope>(json).expect("event deserializes"))
        .collect::<Vec<_>>();

    assert!(frames[0].contains("teammate_event"));
    assert!(frames[0].contains("text_delta"));
    assert!(frames[1].contains("local-agent"));
    assert!(frames[2].contains("waiting"));
    assert!(frames[3].contains("message_sent"));
    assert!(frames[4].contains("completed"));
    assert!(round_trips.iter().all(|frame| frame.id() == "teammate-1"));
}

fn teammate_event_frames() -> [BridgeEnvelope; 5] {
    [
        event(BridgeTeammateEvent::TextDelta {
            delta: "hello".to_owned(),
        }),
        event(BridgeTeammateEvent::Progress {
            token_count: 11,
            tool_use_count: 2,
            last_tool: Some("Read".to_owned()),
            model_id: Some("local-agent".to_owned()),
            cost_usd: Some(0.001),
        }),
        event(BridgeTeammateEvent::Idle {
            agent_name: Some("Variant Agent".to_owned()),
            reason: Some("waiting".to_owned()),
            summary: Some("ready".to_owned()),
        }),
        event(BridgeTeammateEvent::MessageSent {
            from: "reviewer@alpha".to_owned(),
            to: "team-lead".to_owned(),
            text: "done".to_owned(),
            summary: None,
        }),
        event(BridgeTeammateEvent::Completed),
    ]
}

fn event(event: BridgeTeammateEvent) -> BridgeEnvelope {
    BridgeEnvelope::response("teammate-1", BridgeResponse::TeammateEvent { event })
}

#[test]
fn bridge_teammate_mailbox_helpers_round_trip_over_jsonl() {
    let poll = BridgeEnvelope::request(
        "mailbox-1",
        BridgeRequest::TeammateMailboxPoll {
            request: BridgeMailboxPollRequest::unread()
                .with_agent_name("reviewer")
                .with_team_name("alpha")
                .mark_read(true),
        },
    );
    let messages = BridgeEnvelope::response(
        "mailbox-1",
        BridgeResponse::TeammateMailboxMessages {
            messages: vec![BridgeMailboxMessage {
                from: "team-lead".to_owned(),
                text: "please inspect".to_owned(),
                timestamp: "2026-06-27T00:00:00Z".to_owned(),
                color: None,
                summary: Some("inspect".to_owned()),
                read: false,
            }],
        },
    );
    let send = BridgeEnvelope::request(
        "mailbox-2",
        BridgeRequest::TeammateMailboxSend {
            request: BridgeMailboxSendRequest::new("team-lead", "done")
                .with_from("reviewer")
                .with_team_name("alpha")
                .with_summary("complete"),
        },
    );
    let ready = BridgeEnvelope::request(
        "ready-1",
        BridgeRequest::TeammateReady {
            ready: BridgeTeammateReady::new()
                .with_reason("waiting")
                .with_summary("ready for more"),
        },
    );

    let frames = [poll, messages, send, ready]
        .into_iter()
        .map(|frame| serde_json::to_string(&frame).expect("frame serializes"))
        .collect::<Vec<_>>();
    let round_trips = frames
        .iter()
        .map(|json| serde_json::from_str::<BridgeEnvelope>(json).expect("frame deserializes"))
        .collect::<Vec<_>>();

    assert!(frames[0].contains("teammate_mailbox_poll"));
    assert!(frames[1].contains("teammate_mailbox_messages"));
    assert!(frames[2].contains("teammate_mailbox_send"));
    assert!(frames[3].contains("teammate_ready"));
    assert_eq!(round_trips[0].id(), "mailbox-1");
    assert_eq!(round_trips[3].id(), "ready-1");
}
