//! `BetaAgentService` — managed-agents lifecycle.
//!
//! Endpoints (all under `/v1/beta/agents`, with `anthropic-beta:
//! managed-agents-2026-04-01`):
//! - `POST /v1/beta/agents` — create
//! - `GET /v1/beta/agents` — list (paginated)
//! - `GET /v1/beta/agents/{id}` — retrieve
//! - `PATCH /v1/beta/agents/{id}` — update
//! - `DELETE /v1/beta/agents/{id}` — archive
//! - `GET /v1/beta/agents/{id}/versions` — list versions

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use crate::pagination::{ListParams, Page};
use reqwest::Method;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct AgentCreateParams {
    pub name: String,
    pub description: String,
    pub model: String,
    pub system_prompt: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tools: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub skills: Vec<SkillRef>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub mcp_toolsets: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_config: Option<DefaultToolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillRef {
    Anthropic {
        skill_id: String,
        version: Option<String>,
    },
    Custom {
        skill_id: String,
        version: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultToolConfig {
    pub enabled: Option<bool>,
    pub permission_policy: Option<PermissionPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionPolicy {
    AlwaysAllow,
    AlwaysAsk,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub description: String,
    pub model: String,
    pub system_prompt: String,
    pub version: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentList {
    pub data: Vec<Agent>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentUpdateParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<SkillRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_toolsets: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_config: Option<DefaultToolConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentVersion {
    pub id: String,
    pub agent_id: String,
    pub version: u32,
    pub created_at: String,
}

pub struct AgentService {
    client: Client,
}

impl AgentService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn create(&self, params: AgentCreateParams) -> Result<Agent> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, "/v1/beta/agents", Some(beta::MANAGED_AGENTS))
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, agent_id: &str) -> Result<Agent> {
        let path = format!("/v1/beta/agents/{agent_id}");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn list(&self) -> Result<AgentList> {
        let page = self.list_page(&ListParams::default()).await?;
        Ok(AgentList {
            data: page.data,
            has_more: page.has_more,
        })
    }

    pub async fn list_page(&self, params: &ListParams) -> Result<Page<Agent>> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, "/v1/beta/agents", Some(beta::MANAGED_AGENTS))
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update(&self, agent_id: &str, params: AgentUpdateParams) -> Result<Agent> {
        let path = format!("/v1/beta/agents/{agent_id}");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::PATCH, &path, Some(beta::MANAGED_AGENTS))
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn archive(&self, agent_id: &str) -> Result<()> {
        let path = format!("/v1/beta/agents/{agent_id}");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }

    pub async fn list_versions(&self, agent_id: &str) -> Result<Page<AgentVersion>> {
        let path = format!("/v1/beta/agents/{agent_id}/versions");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }
}
