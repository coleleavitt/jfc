//! Pairing tokens and HMAC frame attestation.
//!
//! Each remote-control session generates a random pairing token. The token is
//! shared out-of-band (printed to the terminal, copied via `/remote-control`).
//! Every `RemoteFrame` is signed with an HMAC-SHA256 derived from this token,
//! so a relay or man-in-the-middle cannot forge events. Sequence numbers are
//! monotonic per direction to prevent replay attacks.

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use hmac::{Hmac, KeyInit, Mac};
use rand::Rng;
use sha2::Sha256;

use crate::protocol::RemoteFrame;

type HmacSha256 = Hmac<Sha256>;

/// Length of the raw pairing token in bytes.
const TOKEN_LEN: usize = 32;

/// Generate a cryptographically random pairing token, returned as a
/// base64-encoded string (44 chars for 32 bytes).
pub fn generate_token() -> String {
    let mut buf = [0u8; TOKEN_LEN];
    rand::thread_rng().fill(&mut buf);
    B64.encode(buf)
}

/// Compute the HMAC-SHA256 over a frame's canonical signing input.
///
/// The signing input is `"{version}.{seq}.{ts_ms}.{payload_json}"`, matching
/// [`RemoteFrame::signing_input`].
pub fn sign_frame(token: &str, version: u8, seq: u64, ts_ms: u64, payload_json: &str) -> String {
    let input = RemoteFrame::signing_input(version, seq, ts_ms, payload_json);
    let key = B64
        .decode(token)
        .unwrap_or_else(|_| token.as_bytes().to_vec());
    let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key length");
    mac.update(input.as_bytes());
    B64.encode(mac.finalize().into_bytes())
}

/// Verify a frame's HMAC. Returns `true` if the signature is valid.
///
/// Uses constant-time comparison internally (via `hmac::Mac::verify`).
pub fn verify_frame(token: &str, frame: &RemoteFrame) -> bool {
    let payload_json = match serde_json::to_string(&frame.payload) {
        Ok(j) => j,
        Err(_) => return false,
    };
    let expected = sign_frame(token, frame.version, frame.seq, frame.ts_ms, &payload_json);
    // Constant-time comparison: both are base64 strings of the same length.
    constant_time_eq(expected.as_bytes(), frame.hmac.as_bytes())
}

/// Build a signed `RemoteFrame` from a payload.
pub fn build_signed_frame(
    token: &str,
    seq: u64,
    payload: crate::protocol::RemoteEnvelope,
) -> RemoteFrame {
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let payload_json =
        serde_json::to_string(&payload).expect("RemoteEnvelope is always serializable");
    let hmac = sign_frame(
        token,
        crate::protocol::PROTOCOL_VERSION,
        seq,
        ts_ms,
        &payload_json,
    );
    RemoteFrame {
        version: crate::protocol::PROTOCOL_VERSION,
        seq,
        ts_ms,
        payload,
        hmac,
    }
}

/// Constant-time byte comparison. Falls back to iterative XOR when lengths
/// differ (always returns `false` for different-length inputs).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Tracks the last accepted sequence number from a peer. Rejects frames
/// whose `seq` is not strictly greater than the last accepted.
#[derive(Debug, Default)]
pub struct SeqTracker {
    last_seq: Option<u64>,
}

impl SeqTracker {
    pub fn new() -> Self {
        Self { last_seq: None }
    }

    /// Accept a frame's sequence number. Returns `true` if the seq is
    /// valid (strictly greater than the last accepted); `false` if it's
    /// a replay or out-of-order.
    pub fn accept(&mut self, seq: u64) -> bool {
        match self.last_seq {
            None => {
                self.last_seq = Some(seq);
                true
            }
            Some(last) if seq > last => {
                self.last_seq = Some(seq);
                true
            }
            _ => false,
        }
    }

    /// The last accepted sequence number, if any.
    pub fn last(&self) -> Option<u64> {
        self.last_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::RemoteEnvelope;

    #[test]
    fn token_generation_is_44_chars() {
        let t = generate_token();
        assert_eq!(t.len(), 44); // 32 bytes → 44 base64 chars
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
        assert!(!t.accept(1)); // replay
        assert!(!t.accept(0)); // older
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
}
