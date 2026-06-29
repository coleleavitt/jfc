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
    let _linkscope_token = linkscope::phase("remote.auth.generate_token");
    let mut buf = [0u8; TOKEN_LEN];
    rand::thread_rng().fill(&mut buf);
    let token = B64.encode(buf);
    linkscope::record_bytes(
        "remote.auth.token.bytes",
        usize_to_u64_saturating(token.len()),
    );
    token
}

/// Compute the HMAC-SHA256 over a frame's canonical signing input.
///
/// The signing input is `"{version}.{seq}.{ts_ms}.{payload_json}"`, matching
/// [`RemoteFrame::signing_input`].
pub fn sign_frame(token: &str, version: u8, seq: u64, ts_ms: u64, payload_json: &str) -> String {
    let _linkscope_sign = linkscope::phase("remote.auth.sign_frame");
    let input = RemoteFrame::signing_input(version, seq, ts_ms, payload_json);
    let decoded = B64.decode(token);
    let (key, key_source) = match decoded {
        Ok(key) => (key, "base64"),
        Err(_) => (token.as_bytes().to_vec(), "raw"),
    };
    trace_signing_input(SigningTrace {
        label: "remote.auth.sign_frame.detail",
        version,
        seq,
        payload_bytes: payload_json.len(),
        key_bytes: key.len(),
        key_source,
    });
    let Ok(mut mac) = HmacSha256::new_from_slice(&key) else {
        linkscope::record_items("remote.auth.sign_frame.key_error", 1);
        return String::new();
    };
    mac.update(input.as_bytes());
    let hmac = B64.encode(mac.finalize().into_bytes());
    linkscope::record_bytes(
        "remote.auth.hmac.bytes",
        usize_to_u64_saturating(hmac.len()),
    );
    hmac
}

/// Verify a frame's HMAC. Returns `true` if the signature is valid.
///
/// Uses constant-time comparison internally (via `hmac::Mac::verify`).
pub fn verify_frame(token: &str, frame: &RemoteFrame) -> bool {
    let _linkscope_verify = linkscope::phase("remote.auth.verify_frame");
    let payload_json = match serde_json::to_string(&frame.payload) {
        Ok(j) => j,
        Err(_) => {
            linkscope::record_items("remote.auth.verify_frame.serialize_error", 1);
            return false;
        }
    };
    let expected = sign_frame(token, frame.version, frame.seq, frame.ts_ms, &payload_json);
    // Constant-time comparison: both are base64 strings of the same length.
    let ok = constant_time_eq(expected.as_bytes(), frame.hmac.as_bytes());
    linkscope::record_items(
        if ok {
            "remote.auth.verify_frame.ok"
        } else {
            "remote.auth.verify_frame.failed"
        },
        1,
    );
    trace_verify_result(frame, ok);
    ok
}

/// Build a signed `RemoteFrame` from a payload.
pub fn build_signed_frame(
    token: &str,
    seq: u64,
    payload: crate::protocol::RemoteEnvelope,
) -> RemoteFrame {
    let _linkscope_build = linkscope::phase("remote.auth.build_signed_frame");
    let ts_ms = millis_to_u64_saturating(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    );
    let payload_json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_owned());
    trace_build_frame(BuildFrameTrace {
        seq,
        payload_kind: payload.kind(),
        payload_bytes: payload_json.len(),
    });
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
        linkscope::record_items("remote.auth.seq_tracker.new", 1);
        Self { last_seq: None }
    }

    /// Accept a frame's sequence number. Returns `true` if the seq is
    /// valid (strictly greater than the last accepted); `false` if it's
    /// a replay or out-of-order.
    pub fn accept(&mut self, seq: u64) -> bool {
        let _linkscope_accept = linkscope::phase("remote.auth.seq_tracker.accept");
        let accepted = match self.last_seq {
            None => {
                self.last_seq = Some(seq);
                true
            }
            Some(last) if seq > last => {
                self.last_seq = Some(seq);
                true
            }
            _ => false,
        };
        linkscope::record_items(
            if accepted {
                "remote.auth.seq_tracker.accepted"
            } else {
                "remote.auth.seq_tracker.rejected"
            },
            1,
        );
        trace_seq_accept(SeqTrace {
            seq,
            last_seq: self.last_seq,
            accepted,
        });
        accepted
    }

    /// The last accepted sequence number, if any.
    pub fn last(&self) -> Option<u64> {
        self.last_seq
    }
}

struct SigningTrace<'a> {
    label: &'static str,
    version: u8,
    seq: u64,
    payload_bytes: usize,
    key_bytes: usize,
    key_source: &'a str,
}

struct BuildFrameTrace {
    seq: u64,
    payload_kind: &'static str,
    payload_bytes: usize,
}

struct SeqTrace {
    seq: u64,
    last_seq: Option<u64>,
    accepted: bool,
}

fn trace_signing_input(input: SigningTrace<'_>) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::count("version", u64::from(input.version)),
            linkscope::TraceField::count("seq", input.seq),
            linkscope::TraceField::bytes(
                "payload_bytes",
                usize_to_u64_saturating(input.payload_bytes),
            ),
            linkscope::TraceField::bytes("key_bytes", usize_to_u64_saturating(input.key_bytes)),
            linkscope::TraceField::text("key_source", input.key_source),
        ],
    );
}

fn trace_verify_result(frame: &RemoteFrame, ok: bool) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "remote.auth.verify_frame.detail",
        [
            linkscope::TraceField::count("version", u64::from(frame.version)),
            linkscope::TraceField::count("seq", frame.seq),
            linkscope::TraceField::text("payload_kind", frame.payload.kind()),
            linkscope::TraceField::count("ok", u64::from(ok)),
        ],
    );
}

fn trace_build_frame(input: BuildFrameTrace) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "remote.auth.build_signed_frame.detail",
        [
            linkscope::TraceField::count("seq", input.seq),
            linkscope::TraceField::text("payload_kind", input.payload_kind),
            linkscope::TraceField::bytes(
                "payload_bytes",
                usize_to_u64_saturating(input.payload_bytes),
            ),
        ],
    );
}

fn trace_seq_accept(input: SeqTrace) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "remote.auth.seq_tracker.detail",
        [
            linkscope::TraceField::count("seq", input.seq),
            linkscope::TraceField::count("last_seq", input.last_seq.unwrap_or_default()),
            linkscope::TraceField::count("has_last_seq", u64::from(input.last_seq.is_some())),
            linkscope::TraceField::count("accepted", u64::from(input.accepted)),
        ],
    );
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn millis_to_u64_saturating(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
