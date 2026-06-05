//! End-to-end remote-control protocol tests over both transports.

use jfc_remote::auth::{self, SeqTracker};
use jfc_remote::protocol::{RemoteEnvelope, SessionState};
use jfc_remote::transport::loopback;
use jfc_remote::ws::{WsServer, connect};

/// Full host↔client cycle over the in-memory loopback transport.
#[tokio::test]
async fn loopback_full_cycle() {
    let token = auth::generate_token();
    let ((host_tx, mut host_rx), (client_tx, mut client_rx)) = loopback();

    // Host emits a sequence of outbound events.
    let envs = [
        RemoteEnvelope::AssistantDelta {
            text: Some("Working on it".into()),
            reasoning: None,
        },
        RemoteEnvelope::ToolUse {
            id: "t1".into(),
            name: "Bash".into(),
            input_preview: Some("ls -la".into()),
        },
        RemoteEnvelope::PermissionRequest {
            tool_use_id: "t1".into(),
            tool_name: "Bash".into(),
            summary: "ls -la".into(),
            diff: None,
        },
    ];
    for (host_seq, env) in (1_u64..).zip(envs) {
        let frame = auth::build_signed_frame(&token, host_seq, env);
        host_tx.send(frame).await.unwrap();
    }

    // Client receives + verifies all three.
    let mut client_tracker = SeqTracker::new();
    for expected_seq in 1..=3 {
        let frame = client_rx.recv().await.unwrap();
        assert!(auth::verify_frame(&token, &frame), "HMAC must verify");
        assert!(client_tracker.accept(frame.seq), "seq must be monotonic");
        assert_eq!(frame.seq, expected_seq);
    }

    // Client responds: approve the tool, then send a prompt.
    let approve = auth::build_signed_frame(
        &token,
        1,
        RemoteEnvelope::ApprovalResponse {
            tool_use_id: "t1".into(),
            approved: true,
        },
    );
    client_tx.send(approve).await.unwrap();

    let prompt = auth::build_signed_frame(
        &token,
        2,
        RemoteEnvelope::UserPrompt {
            text: "now run the tests".into(),
        },
    );
    client_tx.send(prompt).await.unwrap();

    // Host receives + verifies both.
    let mut host_tracker = SeqTracker::new();
    let f1 = host_rx.recv().await.unwrap();
    assert!(auth::verify_frame(&token, &f1));
    assert!(host_tracker.accept(f1.seq));
    assert_eq!(
        f1.payload,
        RemoteEnvelope::ApprovalResponse {
            tool_use_id: "t1".into(),
            approved: true
        }
    );

    let f2 = host_rx.recv().await.unwrap();
    assert!(auth::verify_frame(&token, &f2));
    assert!(host_tracker.accept(f2.seq));
    assert_eq!(
        f2.payload,
        RemoteEnvelope::UserPrompt {
            text: "now run the tests".into()
        }
    );
}

/// Full host↔client cycle over a real localhost WebSocket.
#[tokio::test]
async fn websocket_full_cycle() {
    let token = auth::generate_token();
    let addr = "127.0.0.1:0".parse().unwrap();
    let (server, mut clients) = WsServer::bind(addr, token.clone()).await.unwrap();
    let url = format!("ws://127.0.0.1:{}", server.addr.port());

    let (client_tx, mut client_rx) = connect(&url, &token).await.unwrap();
    let (host_tx, mut host_rx) = clients.recv().await.unwrap();

    // Host → client: status + assistant delta.
    host_tx
        .send(auth::build_signed_frame(
            &token,
            1,
            RemoteEnvelope::SessionStatus {
                status: SessionState::Running,
                message: None,
            },
        ))
        .await
        .unwrap();

    let frame = client_rx.recv().await.unwrap();
    assert!(auth::verify_frame(&token, &frame));
    assert_eq!(
        frame.payload,
        RemoteEnvelope::SessionStatus {
            status: SessionState::Running,
            message: None
        }
    );

    // Client → host: prompt.
    client_tx
        .send(auth::build_signed_frame(
            &token,
            1,
            RemoteEnvelope::UserPrompt {
                text: "hello over the wire".into(),
            },
        ))
        .await
        .unwrap();

    let frame = host_rx.recv().await.unwrap();
    assert!(auth::verify_frame(&token, &frame));
    assert_eq!(
        frame.payload,
        RemoteEnvelope::UserPrompt {
            text: "hello over the wire".into()
        }
    );

    server.shutdown();
}

/// A tampered frame must fail HMAC verification end-to-end.
#[tokio::test]
async fn tampered_frame_rejected_e2e() {
    let token = auth::generate_token();
    let ((host_tx, _host_rx), (_client_tx, mut client_rx)) = loopback();

    let mut frame = auth::build_signed_frame(&token, 1, RemoteEnvelope::Heartbeat);
    // Tamper after signing.
    frame.payload = RemoteEnvelope::UserPrompt {
        text: "malicious".into(),
    };
    host_tx.send(frame).await.unwrap();

    let received = client_rx.recv().await.unwrap();
    assert!(
        !auth::verify_frame(&token, &received),
        "tampered frame must fail verification"
    );
}
