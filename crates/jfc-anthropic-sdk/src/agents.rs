//! `BetaAgentService` — managed-agents lifecycle.
//!
//! Endpoints (all under `/v1/beta/agents`, with `anthropic-beta:
//! managed-agents-2026-04-01`):
//! - `POST /v1/agents?beta=true` — create
//! - `GET /v1/agents?beta=true` — list (paginated)
//! - `GET /v1/agents/{id}?beta=true` — retrieve
//! - `POST /v1/agents/{id}?beta=true` — update
//! - `POST /v1/agents/{id}/archive?beta=true` — archive
//! - `GET /v1/agents/{id}/versions?beta=true` — list versions

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use crate::pagination::{ListParams, Page};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct AgentCreateParams {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub description: String,
    /// Upstream accepts either a string model id or a model_config object. The
    /// legacy JFC surface keeps this as a string for source compatibility.
    pub model: String,
    #[serde(rename = "system")]
    pub system_prompt: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tools: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub skills: Vec<SkillRef>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub mcp_toolsets: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub mcp_servers: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_config: Option<DefaultToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiagent: Option<MultiagentParams>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiagentParams {
    #[serde(rename = "type")]
    pub type_: MultiagentType,
    pub agents: Vec<MultiagentRosterEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiagentType {
    Coordinator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MultiagentRosterEntry {
    AgentId(String),
    Agent {
        #[serde(rename = "type")]
        type_: AgentReferenceType,
        id: String,
        version: Option<u64>,
    },
    SelfRef {
        #[serde(rename = "type")]
        type_: SelfReferenceType,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentReferenceType {
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfReferenceType {
    #[serde(rename = "self")]
    Self_,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub model: serde_json::Value,
    #[serde(default, alias = "system_prompt")]
    pub system: String,
    pub version: u64,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub multiagent: Option<MultiagentResolved>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MultiagentResolved {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub agents: Vec<AgentReference>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentReference {
    pub id: String,
    #[serde(default)]
    pub version: Option<u64>,
    #[serde(rename = "type")]
    pub type_: String,
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
    #[serde(rename = "system", skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<SkillRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_toolsets: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_config: Option<DefaultToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiagent: Option<MultiagentParams>,
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
                    .request(
                        Method::POST,
                        "/v1/agents?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, agent_id: &str) -> Result<Agent> {
        let path = format!("/v1/agents/{agent_id}?beta=true");
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
                    .request(
                        Method::GET,
                        "/v1/agents?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update(&self, agent_id: &str, params: AgentUpdateParams) -> Result<Agent> {
        let path = format!("/v1/agents/{agent_id}?beta=true");
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

    pub async fn archive(&self, agent_id: &str) -> Result<Agent> {
        let path = format!("/v1/agents/{agent_id}/archive?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn list_versions(&self, agent_id: &str) -> Result<Page<AgentVersion>> {
        let path = format!("/v1/agents/{agent_id}/versions?beta=true");
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
