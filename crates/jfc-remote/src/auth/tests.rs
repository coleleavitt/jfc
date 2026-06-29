use super::*;
use crate::protocol::RemoteEnvelope;

#[test]
fn token_generation_is_44_chars() {
    let t = generate_token();
    assert_eq!(t.len(), 44);
}

#[test]
fn tokens_are_unique() {
    let a = generate_token();
    let b = generate_token();
    assert_ne!(a, b);
}

#[test]
fn sign_and_verify_roundtrip() {
    let token = generate_token();
    let payload = RemoteEnvelope::Heartbeat;
    let frame = build_signed_frame(&token, 1, payload);
    assert!(verify_frame(&token, &frame));
}

#[test]
fn tampered_payload_rejected() {
    let token = generate_token();
    let frame = build_signed_frame(&token, 1, RemoteEnvelope::Heartbeat);
    let mut tampered = frame;
    tampered.payload = RemoteEnvelope::Ping;
    assert!(!verify_frame(&token, &tampered));
}

#[test]
fn tampered_seq_rejected() {
    let token = generate_token();
    let frame = build_signed_frame(&token, 1, RemoteEnvelope::Heartbeat);
    let mut tampered = frame;
    tampered.seq = 999;
    assert!(!verify_frame(&token, &tampered));
}

#[test]
fn wrong_token_rejected() {
    let token_a = generate_token();
    let token_b = generate_token();
    let frame = build_signed_frame(&token_a, 1, RemoteEnvelope::Heartbeat);
    assert!(!verify_frame(&token_b, &frame));
}

#[test]
fn seq_tracker_accepts_monotonic() {
    let mut t = SeqTracker::new();
    assert!(t.accept(1));
    assert!(t.accept(2));
    assert!(t.accept(5));
}

#[test]
fn seq_tracker_rejects_replay() {
    let mut t = SeqTracker::new();
    assert!(t.accept(1));
    assert!(!t.accept(1));
    assert!(!t.accept(0));
}

#[test]
fn seq_tracker_rejects_equal() {
    let mut t = SeqTracker::new();
    assert!(t.accept(3));
    assert!(!t.accept(3));
}

#[test]
fn constant_time_eq_works() {
    assert!(constant_time_eq(b"hello", b"hello"));
    assert!(!constant_time_eq(b"hello", b"world"));
    assert!(!constant_time_eq(b"hello", b"hell"));
}

#[test]
fn auth_trace_records_shape_without_token_material_normal() {
    linkscope::trace_detail_enable();
    let token = generate_token();
    let frame = build_signed_frame(
        &token,
        7,
        RemoteEnvelope::UserPrompt {
            text: "do not trace this prompt".into(),
        },
    );
    assert!(verify_frame(&token, &frame));

    let snapshot = linkscope::snapshot();
    let rendered = format!("{snapshot:?}");
    assert!(rendered.contains("remote.auth.sign_frame.detail"));
    assert!(rendered.contains("remote.auth.verify_frame.detail"));
    assert!(rendered.contains("user_prompt"));
    assert!(!rendered.contains(&token));
    assert!(!rendered.contains("do not trace this prompt"));
}
