//! Client wiring for Anthropic's managed-agents Sessions API.
//!
//! When the user launches `jfc --remote-session <id>`, this module opens
//! the SSE event stream against `/v1/beta/sessions/{id}/events/stream`,
//! feeds each `SessionEvent` into the transcript, and relays user input
//! back via `send_user_message`.
//!
//! This is the entry point; the rendering side lives in `render.rs`
//! (see `render_session_event`) and the input side stays in `input.rs`.
//! Today this surface is *behind a CLI flag and not the default* —
//! local jfc sessions still go through `provider.rs`.
//!
//! # Lifecycle
//!
//! 1. `connect(session_id)` returns an SSE stream of decoded events.
//! 2. The caller forwards each event to the UI via the AppEvent channel.
//! 3. User input becomes a `send_user_message` call instead of a local
//!    `provider.stream`.

use futures::stream::Stream;
use jfc_anthropic_sdk::sessions::{SessionEvent, SessionResource, SessionService};
use jfc_anthropic_sdk::{Client, Result};
use std::pin::Pin;

pub struct ManagedSession {
    service: SessionService,
    session_id: String,
}

impl ManagedSession {
    pub fn new(client: Client, session_id: impl Into<String>) -> Self {
        Self {
            service: SessionService::new(client),
            session_id: session_id.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.session_id
    }

    /// Connect to the live event stream. Yields decoded `SessionEvent`s
    /// until the server closes the connection.
    pub async fn connect(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<SessionEvent>> + Send>>> {
        self.service.stream_events(&self.session_id).await
    }

    /// Forward a user-typed prompt to the session. The agent's response
    /// arrives via the event stream returned by `connect`.
    pub async fn send(&self, content: serde_json::Value) -> Result<()> {
        self.service
            .send_user_message(&self.session_id, content)
            .await
    }

    /// Attach a Resource (file or repo) to the session. Mirrors v132's
    /// `BetaSessionsResourcesService.Add`.
    pub async fn attach_resource(
        &self,
        resource: jfc_anthropic_sdk::sessions::ResourceRef,
    ) -> Result<SessionResource> {
        self.service.add_resource(&self.session_id, resource).await
    }
}

/// Render a `SessionEvent` as a single line of plain text suitable for
/// the transcript. Full UI integration (color, badges, click handlers)
/// is still TODO — this is the bare-minimum text fallback so callers
/// can append to messages without a full type-aware renderer.
pub fn render_event_line(event: &SessionEvent) -> String {
    match event {
        SessionEvent::UserMessage { content, .. } => format!("[user] {content}"),
        SessionEvent::AgentMessage { content, .. } => format!("[agent] {content}"),
        SessionEvent::AgentThinking { .. } => "[agent thinking]".to_owned(),
        SessionEvent::AgentToolUse { name, .. } => format!("[agent → tool: {name}]"),
        SessionEvent::AgentMcpToolUse { server, name, .. } => {
            format!("[agent → mcp: {server}/{name}]")
        }
        SessionEvent::AgentToolResult { tool_use_id, .. } => {
            format!("[tool result: {tool_use_id}]")
        }
        SessionEvent::AgentMcpToolResult { tool_use_id, .. } => {
            format!("[mcp result: {tool_use_id}]")
        }
        SessionEvent::AgentCustomToolUse { name, .. } => {
            format!("[agent → custom: {name}]")
        }
        SessionEvent::UserCustomToolResult { tool_use_id, .. } => {
            format!("[user custom result: {tool_use_id}]")
        }
        SessionEvent::UserToolConfirmation {
            tool_use_id,
            approved,
            ..
        } => {
            format!("[user confirmation: {tool_use_id} → {approved}]")
        }
        SessionEvent::AgentThreadContextCompacted { .. } => "[context compacted]".to_owned(),
        SessionEvent::SessionStatusRunning { .. } => "[session: running]".to_owned(),
        SessionEvent::SessionStatusIdle { .. } => "[session: idle]".to_owned(),
        SessionEvent::SessionStatusTerminated { reason, .. } => match reason {
            Some(r) => format!("[session: terminated — {r}]"),
            None => "[session: terminated]".to_owned(),
        },
        SessionEvent::SessionStatusRescheduled { .. } => "[session: rescheduled]".to_owned(),
        SessionEvent::SessionDeleted { .. } => "[session: deleted]".to_owned(),
        SessionEvent::SessionError { message, .. } => format!("[session error: {message}]"),
        SessionEvent::SpanModelRequestEnd { duration_ms, .. } => {
            format!("[span: model request {duration_ms}ms]")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_event_line_user_message_normal() {
        let ev = SessionEvent::UserMessage {
            content: json!({"text": "hello"}),
            timestamp: "2026-05-07T00:00:00Z".into(),
        };
        let s = render_event_line(&ev);
        assert!(s.starts_with("[user]"));
    }

    #[test]
    fn render_event_line_session_terminated_with_reason_normal() {
        let ev = SessionEvent::SessionStatusTerminated {
            reason: Some("budget".into()),
            timestamp: "t".into(),
        };
        assert_eq!(render_event_line(&ev), "[session: terminated — budget]");
    }

    #[test]
    fn render_event_line_session_terminated_no_reason_robust() {
        let ev = SessionEvent::SessionStatusTerminated {
            reason: None,
            timestamp: "t".into(),
        };
        assert_eq!(render_event_line(&ev), "[session: terminated]");
    }
}
