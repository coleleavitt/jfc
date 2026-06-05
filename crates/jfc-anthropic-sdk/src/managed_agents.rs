//! Managed Agents high-level API — composite operations for the common
//! agent lifecycle: create an agent, create a session, send/stream events.
//!
//! This module wraps the lower-level `agents`, `sessions`, and `environments`
//! services into a single ergonomic surface that callers can use without
//! juggling three service handles.

use crate::agents::{Agent, AgentCreateParams, AgentService};
use crate::beta;
use crate::client::Client;
use crate::environments::{Environment, EnvironmentCreateParams, EnvironmentService};
use crate::error::Result;
use crate::sessions::{Session, SessionCreateParams, SessionEvent, SessionService, SessionState};
use futures::stream::Stream;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

/// Composite agent descriptor combining identity, model, system prompt,
/// tools, and versioning into a single view.
#[derive(Debug, Clone, Deserialize)]
pub struct ManagedAgent {
    pub id: String,
    pub name: String,
    pub model: String,
    pub system: String,
    pub tools: Vec<serde_json::Value>,
    pub version: u32,
}

impl From<Agent> for ManagedAgent {
    fn from(a: Agent) -> Self {
        let model = match a.model {
            serde_json::Value::String(model) => model,
            other => other
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_owned(),
        };
        Self {
            id: a.id,
            name: a.name,
            model,
            system: a.system,
            tools: Vec::new(), // tools are stored separately in AgentCreateParams
            version: a.version as u32,
        }
    }
}

/// Environment descriptor — isolated execution context for sessions.
#[derive(Debug, Clone, Deserialize)]
pub struct ManagedEnvironment {
    pub id: String,
    pub name: String,
    pub config: serde_json::Value,
}

impl From<Environment> for ManagedEnvironment {
    fn from(e: Environment) -> Self {
        Self {
            id: e.id,
            name: e.name,
            config: serde_json::json!({ "created_at": e.created_at }),
        }
    }
}

/// Session with usage tracking.
#[derive(Debug, Clone, Deserialize)]
pub struct ManagedSession {
    pub id: String,
    pub agent_id: String,
    pub environment_id: Option<String>,
    pub status: ManagedSessionStatus,
    pub usage: SessionUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSessionStatus {
    Idle,
    Running,
    Terminated,
}

impl From<SessionState> for ManagedSessionStatus {
    fn from(s: SessionState) -> Self {
        match s {
            SessionState::Idle => Self::Idle,
            SessionState::Running => Self::Running,
            SessionState::Terminated => Self::Terminated,
            SessionState::Rescheduling => Self::Running,
            SessionState::Rescheduled => Self::Running,
            SessionState::RequiresAction => Self::Idle,
        }
    }
}

/// Token/cost usage for a session.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SessionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
}

/// An event in a session's event stream (simplified view).
#[derive(Debug, Clone, Deserialize)]
pub struct ManagedSessionEvent {
    #[serde(rename = "type")]
    pub type_: String,
    pub content: serde_json::Value,
    pub id: Option<String>,
}

/// Params for sending events into a session.
#[derive(Debug, Clone, Serialize)]
pub struct SendEventsParams {
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_results: Option<Vec<serde_json::Value>>,
}

/// High-level managed agents client.
pub struct ManagedAgentsClient {
    client: Client,
    agent_service: AgentService,
    session_service: SessionService,
    environment_service: EnvironmentService,
}

impl ManagedAgentsClient {
    pub fn new(client: Client) -> Self {
        Self {
            agent_service: AgentService::new(client.clone()),
            session_service: SessionService::new(client.clone()),
            environment_service: EnvironmentService::new(client.clone()),
            client,
        }
    }

    /// Create a new managed agent.
    pub async fn create_agent(&self, params: AgentCreateParams) -> Result<ManagedAgent> {
        let agent = self.agent_service.create(params).await?;
        Ok(agent.into())
    }

    /// Create a new environment for running sessions.
    pub async fn create_environment(
        &self,
        params: EnvironmentCreateParams,
    ) -> Result<ManagedEnvironment> {
        let env = self.environment_service.create(params).await?;
        Ok(env.into())
    }

    /// Create a new session for an agent.
    pub async fn create_session(&self, agent_id: String) -> Result<ManagedSession> {
        let params = SessionCreateParams::for_agent(agent_id);
        let session = self.session_service.create(params).await?;
        Ok(ManagedSession {
            id: session.id,
            agent_id: session.agent_id,
            environment_id: session.environment_id,
            status: session.state.into(),
            usage: SessionUsage::default(),
        })
    }

    /// Create a new session in a self-hosted/managed environment.
    pub async fn create_session_in_environment(
        &self,
        agent_id: String,
        environment_id: String,
    ) -> Result<ManagedSession> {
        let mut params = SessionCreateParams::for_agent(agent_id);
        params.environment_id = Some(environment_id);
        let session = self.session_service.create(params).await?;
        Ok(ManagedSession {
            id: session.id,
            agent_id: session.agent_id,
            environment_id: session.environment_id,
            status: session.state.into(),
            usage: SessionUsage::default(),
        })
    }

    /// Send events (user message / tool results) into a session.
    pub async fn send_events(&self, session_id: &str, params: SendEventsParams) -> Result<()> {
        self.session_service
            .send_user_message(session_id, params.content)
            .await
    }

    /// Stream events from a session via SSE.
    pub async fn stream_events(
        &self,
        session_id: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<SessionEvent>> + Send>>> {
        self.session_service.stream_events(session_id).await
    }

    /// List all sessions (optionally filtered by agent).
    pub async fn list_sessions(&self, agent_id: Option<&str>) -> Result<Vec<ManagedSession>> {
        // Use the sessions list endpoint with optional agent_id filter
        let path = match agent_id {
            Some(aid) => format!("/v1/sessions?beta=true&agent_id={aid}"),
            None => "/v1/sessions?beta=true".to_string(),
        };
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

        // Parse raw sessions and convert
        let raw_sessions: Vec<Session> = serde_json::from_value(data)
            .map_err(|e| crate::error::Error::Stream(format!("parse sessions: {e}")))?;

        Ok(raw_sessions
            .into_iter()
            .map(|s| ManagedSession {
                id: s.id,
                agent_id: s.agent_id,
                environment_id: s.environment_id,
                status: s.state.into(),
                usage: SessionUsage::default(),
            })
            .collect())
    }
}
