//! `BetaSessionService` — persistent agent sessions, resources, and event stream.
//!
//! Endpoints (beta `managed-agents-2026-04-01`):
//! - `POST /v1/sessions?beta=true` — create
//! - `GET /v1/sessions?beta=true` — list
//! - `GET /v1/sessions/{id}?beta=true` — retrieve
//! - `POST /v1/sessions/{id}?beta=true` — update
//! - `DELETE /v1/sessions/{id}?beta=true` — delete
//! - `POST /v1/sessions/{id}/archive?beta=true` — archive
//! - `POST /v1/sessions/{id}/resources?beta=true` — attach a resource
//! - `GET /v1/sessions/{id}/events?beta=true` — list events
//! - `POST /v1/sessions/{id}/events?beta=true` — send a user event
//! - `GET /v1/sessions/{id}/events/stream?beta=true` — SSE event stream

use crate::beta;
use crate::client::Client;
use crate::error::{Error, Result};
use crate::pagination::{ListParams, Page};
use futures::stream::{Stream, StreamExt};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::pin::Pin;

#[derive(Debug, Clone, Default, Serialize)]
pub struct SessionCreateParams {
    /// Upstream accepts an agent id string or `{ id, version }`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<serde_json::Value>,
    /// Legacy JFC compatibility. New callers should use `agent`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub resources: Vec<ResourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub vault_ids: Vec<String>,
}

impl SessionCreateParams {
    pub fn for_agent(agent_id: impl Into<String>) -> Self {
        Self {
            agent: Some(serde_json::Value::String(agent_id.into())),
            ..Self::default()
        }
    }
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
    MemoryStore {
        memory_store_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        access: Option<MemoryStoreAccess>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStoreAccess {
    ReadWrite,
    ReadOnly,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub agent: serde_json::Value,
    #[serde(default, alias = "state", alias = "status")]
    pub state: SessionState,
    #[serde(default)]
    pub environment_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub outcome_evaluations: Vec<OutcomeEvaluation>,
    #[serde(default)]
    pub vault_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutcomeEvaluation {
    pub outcome_id: String,
    pub description: String,
    pub result: String,
    #[serde(default)]
    pub explanation: Option<String>,
    #[serde(default)]
    pub iteration: Option<u64>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    #[default]
    Idle,
    Running,
    Terminated,
    Rescheduling,
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
    SpanOutcomeEvaluationStart {
        #[serde(default)]
        outcome_id: Option<String>,
        timestamp: String,
    },
    SpanOutcomeEvaluationOngoing {
        #[serde(default)]
        outcome_id: Option<String>,
        timestamp: String,
    },
    SpanOutcomeEvaluationEnd {
        #[serde(default)]
        outcome_id: Option<String>,
        #[serde(default)]
        result: Option<String>,
        timestamp: String,
    },
    UserDefineOutcome {
        description: String,
        #[serde(default)]
        rubric: serde_json::Value,
        timestamp: String,
    },
    #[serde(other)]
    Unknown,
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
    MemoryStore {
        id: String,
        memory_store_id: String,
        #[serde(default)]
        instructions: Option<String>,
        #[serde(default)]
        access: Option<MemoryStoreAccess>,
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
                        "/v1/sessions?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, session_id: &str) -> Result<Session> {
        let path = format!("/v1/sessions/{session_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn list_page(&self, params: &ListParams) -> Result<Page<Session>> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(
                        Method::GET,
                        "/v1/sessions?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update(&self, session_id: &str, params: serde_json::Value) -> Result<Session> {
        let path = format!("/v1/sessions/{session_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete(&self, session_id: &str) -> Result<()> {
        let path = format!("/v1/sessions/{session_id}?beta=true");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }

    pub async fn archive(&self, session_id: &str) -> Result<Session> {
        let path = format!("/v1/sessions/{session_id}/archive?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    /// Attach a Resource (file or repo) to a managed session.
    pub async fn add_resource(
        &self,
        session_id: &str,
        resource: ResourceRef,
    ) -> Result<SessionResource> {
        let path = format!("/v1/sessions/{session_id}/resources?beta=true");
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
        let path = format!("/v1/sessions/{session_id}/resources?beta=true");
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
        let path = format!("/v1/sessions/{session_id}/events?beta=true");
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
        let path = format!("/v1/sessions/{session_id}/events/stream?beta=true");
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
