#![allow(dead_code, unused_imports)]
//! Session mirrors — real-time observation of teammate work.
//!
//! A mirror allows the leader (or any observer) to see a teammate's
//! tool calls and model responses in real-time, without interfering
//! with their execution.
//!
//! ## Wiring status
//!
//! Data layer is complete (event format, file-watcher, mailbox).
//! UI integration is the missing piece: a leader-side `/mirror <id>`
//! slash command that opens a follower pane, plus a teammate-side
//! emitter that writes mirror events alongside the existing mailbox
//! traffic. Both pieces are tractable but require ratatui pane
//! plumbing the rest of the swarm code doesn't have yet. Marked
//! `dead_code` on purpose — code preserved, wiring deferred.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// A mirror event — streamed to observers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorEvent {
    pub agent_name: String,
    pub event_type: MirrorEventType,
    pub timestamp: u64,
    pub payload: String,
}

/// Types of mirror events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MirrorEventType {
    /// Agent is calling a tool.
    ToolCall { tool_name: String },
    /// Tool returned a result.
    ToolResult { tool_name: String, success: bool },
    /// Model is generating text.
    ModelText { preview: String },
    /// Agent went idle.
    Idle,
    /// Agent completed its task.
    Completed,
    /// Agent encountered an error.
    Error { message: String },
    /// Agent is streaming (partial response).
    Streaming,
}

/// A mirror session — one per observed teammate.
pub struct MirrorSession {
    pub agent_name: String,
    pub tx: broadcast::Sender<MirrorEvent>,
    pub started_at: SystemTime,
}

impl MirrorSession {
    /// Create a new mirror session for an agent.
    pub fn new(agent_name: &str) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            agent_name: agent_name.to_string(),
            tx,
            started_at: SystemTime::now(),
        }
    }

    /// Emit a mirror event (broadcasts to all subscribers).
    pub fn emit(&self, event_type: MirrorEventType, payload: &str) {
        let event = MirrorEvent {
            agent_name: self.agent_name.clone(),
            event_type,
            timestamp: crate::swarm::team_helpers::now_millis(),
            payload: payload.to_string(),
        };
        // Ignore send errors (no subscribers is fine)
        let _ = self.tx.send(event);
    }

    /// Subscribe to this mirror (returns a receiver for events).
    pub fn subscribe(&self) -> broadcast::Receiver<MirrorEvent> {
        self.tx.subscribe()
    }

    /// Shorthand: emit a tool call event.
    pub fn tool_call(&self, tool_name: &str, input_preview: &str) {
        self.emit(
            MirrorEventType::ToolCall {
                tool_name: tool_name.to_string(),
            },
            input_preview,
        );
    }

    /// Shorthand: emit a tool result event.
    pub fn tool_result(&self, tool_name: &str, success: bool, output_preview: &str) {
        self.emit(
            MirrorEventType::ToolResult {
                tool_name: tool_name.to_string(),
                success,
            },
            output_preview,
        );
    }

    /// Shorthand: emit model text event.
    pub fn model_text(&self, text: &str) {
        let preview = if text.len() > 200 {
            format!("{}...", &text[..200])
        } else {
            text.to_string()
        };
        self.emit(
            MirrorEventType::ModelText {
                preview: preview.clone(),
            },
            &preview,
        );
    }

    /// Shorthand: emit idle.
    pub fn idle(&self) {
        self.emit(MirrorEventType::Idle, "");
    }

    /// Shorthand: emit completed.
    pub fn completed(&self, summary: &str) {
        self.emit(MirrorEventType::Completed, summary);
    }
}

/// Registry of active mirror sessions.
pub struct MirrorRegistry {
    sessions: Vec<MirrorSession>,
}

impl MirrorRegistry {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
        }
    }

    /// Start mirroring an agent.
    pub fn start_mirror(&mut self, agent_name: &str) -> broadcast::Receiver<MirrorEvent> {
        // Check if already mirroring
        if let Some(existing) = self.sessions.iter().find(|s| s.agent_name == agent_name) {
            return existing.subscribe();
        }
        let session = MirrorSession::new(agent_name);
        let rx = session.subscribe();
        self.sessions.push(session);
        rx
    }

    /// Stop mirroring an agent.
    pub fn stop_mirror(&mut self, agent_name: &str) {
        self.sessions.retain(|s| s.agent_name != agent_name);
    }

    /// Get a mirror session by agent name (for emitting events).
    pub fn get(&self, agent_name: &str) -> Option<&MirrorSession> {
        self.sessions.iter().find(|s| s.agent_name == agent_name)
    }

    /// List all active mirrors.
    pub fn active_mirrors(&self) -> Vec<&str> {
        self.sessions
            .iter()
            .map(|s| s.agent_name.as_str())
            .collect()
    }
}

impl Default for MirrorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_session_broadcasts() {
        let session = MirrorSession::new("agent-1");
        let mut rx = session.subscribe();

        session.tool_call("bash", "cargo test");
        let event = rx.try_recv().unwrap();
        assert_eq!(event.agent_name, "agent-1");
        assert!(matches!(event.event_type, MirrorEventType::ToolCall { .. }));
    }

    #[test]
    fn mirror_registry_lifecycle() {
        let mut registry = MirrorRegistry::new();
        let _rx = registry.start_mirror("agent-1");
        assert_eq!(registry.active_mirrors().len(), 1);

        registry.stop_mirror("agent-1");
        assert_eq!(registry.active_mirrors().len(), 0);
    }
}
