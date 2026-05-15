//! `BetaSessionService` — persistent agent sessions, resources, and event stream.
//!
//! Endpoints (under `/v1/beta/sessions`, beta `managed-agents-2026-04-01`):
//! - `POST /v1/beta/sessions` — create
//! - `GET /v1/beta/sessions` — list
//! - `GET /v1/beta/sessions/{id}` — retrieve
//! - `PATCH /v1/beta/sessions/{id}` — update
//! - `DELETE /v1/beta/sessions/{id}` — delete
//! - `POST /v1/beta/sessions/{id}/resources` — attach a resource
//! - `GET /v1/beta/sessions/{id}/events` — list events
//! - `POST /v1/beta/sessions/{id}/events` — send a user event
//! - `GET /v1/beta/sessions/{id}/events/stream` — SSE event stream

use crate::beta;
use crate::client::Client;
use crate::error::{Error, Result};
use futures::stream::{Stream, StreamExt};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

#[derive(Debug, Clone, Serialize)]
pub struct SessionCreateParams {
    pub agent_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub resources: Vec<ResourceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResourceRef {
    File {
        file_id: String,
    },
    GithubRepository {
        owner: String,
        repo: String,
        branch: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub state: SessionState,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Running,
    Terminated,
    Rescheduled,
    RequiresAction,
}

/// Per the SDK's `BetaManagedAgentsSessionEventUnion`, sessions multiplex
/// 17+ event types. We expose them as a single tagged enum so callers can
/// pattern-match.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    UserMessage {
        content: serde_json::Value,
        timestamp: String,
    },
    AgentMessage {
        content: serde_json::Value,
        timestamp: String,
    },
    AgentThinking {
        content: serde_json::Value,
        timestamp: String,
    },
    AgentToolUse {
        tool_use_id: String,
        name: String,
        input: serde_json::Value,
        timestamp: String,
    },
    AgentMcpToolUse {
        tool_use_id: String,
        server: String,
        name: String,
        input: serde_json::Value,
        timestamp: String,
    },
    AgentToolResult {
        tool_use_id: String,
        content: serde_json::Value,
        is_error: Option<bool>,
        timestamp: String,
    },
    AgentMcpToolResult {
        tool_use_id: String,
        content: serde_json::Value,
        is_error: Option<bool>,
        timestamp: String,
    },
    AgentCustomToolUse {
        tool_use_id: String,
        name: String,
        input: serde_json::Value,
        timestamp: String,
    },
    UserCustomToolResult {
        tool_use_id: String,
        content: serde_json::Value,
        timestamp: String,
    },
    UserToolConfirmation {
        tool_use_id: String,
        approved: bool,
        timestamp: String,
    },
    AgentThreadContextCompacted {
        timestamp: String,
    },
    SessionStatusRunning {
        timestamp: String,
    },
    SessionStatusIdle {
        timestamp: String,
    },
    SessionStatusTerminated {
        reason: Option<String>,
        timestamp: String,
    },
    SessionStatusRescheduled {
        timestamp: String,
    },
    SessionDeleted {
        timestamp: String,
    },
    SessionError {
        message: String,
        timestamp: String,
    },
    SpanModelRequestEnd {
        duration_ms: u64,
        timestamp: String,
    },
}

/// A Resource attached to a Session (file or git repo). Mirrors v132's
/// `BetaManagedAgentsSessionResourceUnion`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionResource {
    File {
        id: String,
        file_id: String,
        filename: String,
        created_at: String,
    },
    GithubRepository {
        id: String,
        owner: String,
        repo: String,
        branch: Option<String>,
        created_at: String,
    },
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionStats {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms: f64,
    pub tool_call_count: u64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

pub struct SessionService {
    client: Client,
}

impl SessionService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn create(&self, params: SessionCreateParams) -> Result<Session> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(
                        Method::POST,
                        "/v1/beta/sessions",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, session_id: &str) -> Result<Session> {
        let path = format!("/v1/beta/sessions/{session_id}");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete(&self, session_id: &str) -> Result<()> {
        let path = format!("/v1/beta/sessions/{session_id}");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }

    /// Attach a Resource (file or repo) to a managed session.
    pub async fn add_resource(
        &self,
        session_id: &str,
        resource: ResourceRef,
    ) -> Result<SessionResource> {
        let path = format!("/v1/beta/sessions/{session_id}/resources");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
                    .json(&resource)
            })
            .await?;
        Ok(resp.json().await?)
    }

    /// List Resources currently attached to the session.
    pub async fn list_resources(&self, session_id: &str) -> Result<Vec<SessionResource>> {
        let path = format!("/v1/beta/sessions/{session_id}/resources");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        let body: serde_json::Value = resp.json().await?;
        let data = body
            .get("data")
            .cloned()
            .unwrap_or(serde_json::Value::Array(Vec::new()));
        Ok(serde_json::from_value(data)?)
    }

    /// Send a User event into a managed session (text message + optional
    /// file references). The next streamed event from the agent is the
    /// reply.
    pub async fn send_user_message(
        &self,
        session_id: &str,
        content: serde_json::Value,
    ) -> Result<()> {
        let path = format!("/v1/beta/sessions/{session_id}/events");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
                    .json(&serde_json::json!({
                        "type": "user_message",
                        "content": content,
                    }))
            })
            .await?;
        Ok(())
    }

    /// Subscribe to the live event stream for a session. The returned
    /// stream yields decoded `SessionEvent` values until the server
    /// closes the connection (Ended status, deletion, or transport
    /// error). SSE wire shape: `event: <type>\ndata: <json>\n\n`.
    pub async fn stream_events(
        &self,
        session_id: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<SessionEvent>> + Send>>> {
        let path = format!("/v1/beta/sessions/{session_id}/events/stream");
        let resp = self
            .client
            .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            .header("accept", "text/event-stream")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Stream(format!(
                "session event stream returned status {}",
                resp.status()
            )));
        }
        let events = crate::sse::response_event_stream(resp).filter_map(|ev| async move {
            match ev {
                Ok(event) => {
                    if event.data.is_empty() {
                        return None;
                    }
                    match serde_json::from_str::<SessionEvent>(&event.data) {
                        Ok(decoded) => Some(Ok(decoded)),
                        Err(e) => Some(Err(Error::Stream(format!(
                            "decode SessionEvent: {e} (data: {} bytes)",
                            event.data.len()
                        )))),
                    }
                }
                Err(e) => Some(Err(Error::Stream(e.to_string()))),
            }
        });
        Ok(Box::pin(events))
    }
}
