//! Remote-control wire protocol.
//!
//! `RemoteEnvelope` is the serializable payload exchanged between host and
//! client over any transport. It is intentionally **not** `AppEvent` —
//! `AppEvent` carries non-serializable types (`crossterm::Event`, UI-internal
//! `ToolCall`) and leaks implementation details. This protocol is a stable,
//! versioned, public contract.
//!
//! `RemoteFrame` wraps each envelope with a monotonic sequence number,
//! timestamp, and HMAC so frames can be authenticated and replays rejected.

use serde::{Deserialize, Serialize};

/// Bump when the envelope schema changes in a non-backward-compatible way.
pub const PROTOCOL_VERSION: u8 = 1;

/// Default WebSocket port for the remote-control server.
pub const DEFAULT_PORT: u16 = 4242;

// ─── Envelope ────────────────────────────────────────────────────────────────

/// The payload of a remote-control frame. Tagged JSON via serde's
/// `internally_tagged` representation so each variant encodes as
/// `{"type":"assistant_delta","text":"..."}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteEnvelope {
    // ── Outbound (host → client) ─────────────────────────────────────
    /// Streaming assistant text delta.
    AssistantDelta {
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },

    /// The model invoked a tool. `input_preview` is a truncated pretty-print
    /// of the tool input (avoids sending multi-MB Read results over the wire).
    ToolUse {
        id: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        input_preview: Option<String>,
    },

    /// Tool execution finished.
    ToolResult {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_preview: Option<String>,
        is_error: bool,
    },

    /// Update the set of tool ids currently executing on the host. Mirrors
    /// Claude CLI's SDK bridge event `set_in_progress_tool_use_ids`.
    SetInProgressToolUseIds { action: String, ids: Vec<String> },

    /// Tool yielded to the host but deferred before execution (approval,
    /// classifier, or stream_done queue).
    DeferredToolUse {
        id: String,
        name: String,
        input_preview: String,
        reason: String,
    },

    /// Single-line summary label for a completed tool batch.
    ToolUseSummary {
        summary: String,
        preceding_tool_use_ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },

    /// Session lifecycle status change.
    SessionStatus {
        status: SessionState,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Permission-gated tool — the host is waiting for the client to approve.
    PermissionRequest {
        tool_use_id: String,
        tool_name: String,
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
    },

    /// The model wants to exit plan mode — the client should see the plan
    /// and approve/reject.
    PlanApprovalRequest { plan: String },

    /// Non-blocking toast notification.
    Toast { kind: String, text: String },

    /// Keep-alive from host.
    Heartbeat,

    // ── Inbound (client → host) ──────────────────────────────────────
    /// Submit a user prompt.
    UserPrompt { text: String },

    /// Cancel the current turn (equivalent to pressing Escape).
    Interrupt,

    /// Respond to a `PermissionRequest`.
    ApprovalResponse { tool_use_id: String, approved: bool },

    /// Respond to a `PlanApprovalRequest`.
    PlanApprovalResponse {
        approve: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        feedback: Option<String>,
    },

    /// Client keep-alive.
    Ping,
}

/// Session lifecycle state, mirroring the host's operational mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Running,
    Idle,
    WaitingApproval,
    Terminated,
    Error,
}

// ─── Frame ───────────────────────────────────────────────────────────────────

/// A framed envelope: version + sequence + timestamp + payload + HMAC.
///
/// The HMAC covers `"{version}.{seq}.{ts_ms}.{payload_json}"` so tampering
/// with any field — including reordering — is detectable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteFrame {
    /// Protocol version. Clients should reject frames with a version they
    /// don't understand.
    pub version: u8,
    /// Monotonically increasing sequence number per direction. The receiver
    /// must reject frames whose `seq` ≤ the last accepted `seq` from that
    /// peer.
    pub seq: u64,
    /// Wall-clock milliseconds since UNIX epoch. Informational — not used
    /// for ordering (seq is authoritative).
    pub ts_ms: u64,
    /// The actual payload.
    pub payload: RemoteEnvelope,
    /// Base64-encoded HMAC-SHA256 over the canonical `"{ver}.{seq}.{ts}.{payload_json}"`.
    pub hmac: String,
}

impl RemoteFrame {
    /// The canonical string that is HMAC-signed.
    pub fn signing_input(version: u8, seq: u64, ts_ms: u64, payload_json: &str) -> String {
        let _linkscope_signing = linkscope::phase("remote.protocol.signing_input");
        trace_payload_shape(
            "remote.protocol.signing_input.detail",
            version,
            seq,
            payload_json,
        );
        format!("{version}.{seq}.{ts_ms}.{payload_json}")
    }
}

// ─── Direction helpers ───────────────────────────────────────────────────────

impl RemoteEnvelope {
    /// True if this variant is sent by the host to the client.
    pub fn is_outbound(&self) -> bool {
        let outbound = matches!(
            self,
            Self::AssistantDelta { .. }
                | Self::ToolUse { .. }
                | Self::ToolResult { .. }
                | Self::SetInProgressToolUseIds { .. }
                | Self::DeferredToolUse { .. }
                | Self::ToolUseSummary { .. }
                | Self::SessionStatus { .. }
                | Self::PermissionRequest { .. }
                | Self::PlanApprovalRequest { .. }
                | Self::Toast { .. }
                | Self::Heartbeat
        );
        linkscope::record_items(
            if outbound {
                "remote.protocol.envelope.outbound"
            } else {
                "remote.protocol.envelope.not_outbound"
            },
            1,
        );
        trace_envelope_direction("remote.protocol.envelope.direction", self, outbound);
        outbound
    }

    /// True if this variant is sent by the client to the host.
    pub fn is_inbound(&self) -> bool {
        let inbound = !self.is_outbound();
        linkscope::record_items(
            if inbound {
                "remote.protocol.envelope.inbound"
            } else {
                "remote.protocol.envelope.not_inbound"
            },
            1,
        );
        inbound
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::AssistantDelta { .. } => "assistant_delta",
            Self::ToolUse { .. } => "tool_use",
            Self::ToolResult { .. } => "tool_result",
            Self::SetInProgressToolUseIds { .. } => "set_in_progress_tool_use_ids",
            Self::DeferredToolUse { .. } => "deferred_tool_use",
            Self::ToolUseSummary { .. } => "tool_use_summary",
            Self::SessionStatus { .. } => "session_status",
            Self::PermissionRequest { .. } => "permission_request",
            Self::PlanApprovalRequest { .. } => "plan_approval_request",
            Self::Toast { .. } => "toast",
            Self::Heartbeat => "heartbeat",
            Self::UserPrompt { .. } => "user_prompt",
            Self::Interrupt => "interrupt",
            Self::ApprovalResponse { .. } => "approval_response",
            Self::PlanApprovalResponse { .. } => "plan_approval_response",
            Self::Ping => "ping",
        }
    }
}

fn trace_payload_shape(label: &'static str, version: u8, seq: u64, payload_json: &str) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count("version", u64::from(version)),
            linkscope::TraceField::count("seq", seq),
            linkscope::TraceField::bytes("payload_bytes", len_to_u64(payload_json.len())),
        ],
    );
}

fn trace_envelope_direction(label: &'static str, envelope: &RemoteEnvelope, outbound: bool) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::text("kind", envelope.kind()),
            linkscope::TraceField::count("outbound", u64::from(outbound)),
        ],
    );
}

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
