//! `BetaEnvironmentService` — isolated execution contexts.
//!
//! Endpoints (`anthropic-beta: managed-agents-2026-04-01`):
//! - `POST /v1/environments?beta=true` — create
//! - `GET /v1/environments?beta=true` — list
//! - `GET /v1/environments/{id}?beta=true` — retrieve
//! - `POST /v1/environments/{id}?beta=true` — update
//! - `DELETE /v1/environments/{id}?beta=true` — delete
//! - `POST /v1/environments/{id}/archive?beta=true` — archive
//! - `GET/POST /v1/environments/{id}/work...` — self-hosted worker queue

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use crate::pagination::{ListParams, Page};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentCreateParams {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub packages: Vec<PackageSpec>,
}

/// Mirrors the SDK's `BetaPackagesParams`. Each package manager is its own
/// variant so callers can mix Cargo + Pip + Npm in one environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "manager", rename_all = "snake_case")]
pub enum PackageSpec {
    Cargo {
        name: String,
        version: Option<String>,
    },
    Pip {
        name: String,
        version: Option<String>,
    },
    Npm {
        name: String,
        version: Option<String>,
    },
    Gem {
        name: String,
        version: Option<String>,
    },
    Go {
        module: String,
        version: Option<String>,
    },
    Apt {
        name: String,
        version: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Environment {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EnvironmentUpdateParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages: Option<Vec<PackageSpec>>,
}

pub struct EnvironmentService {
    client: Client,
}

impl EnvironmentService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn create(&self, params: EnvironmentCreateParams) -> Result<Environment> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(
                        Method::POST,
                        "/v1/environments?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn list(&self) -> Result<Page<Environment>> {
        self.list_page(&ListParams::default()).await
    }

    pub async fn list_page(&self, params: &ListParams) -> Result<Page<Environment>> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(
                        Method::GET,
                        "/v1/environments?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, environment_id: &str) -> Result<Environment> {
        let path = format!("/v1/environments/{environment_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update(
        &self,
        environment_id: &str,
        params: EnvironmentUpdateParams,
    ) -> Result<Environment> {
        let path = format!("/v1/environments/{environment_id}?beta=true");
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

    pub async fn delete(&self, environment_id: &str) -> Result<()> {
        let path = format!("/v1/environments/{environment_id}?beta=true");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }

    pub async fn archive(&self, environment_id: &str) -> Result<Environment> {
        let path = format!("/v1/environments/{environment_id}/archive?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub fn work(&self) -> EnvironmentWorkService {
        EnvironmentWorkService::new(self.client.clone())
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkPollParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reclaim_older_than_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkHeartbeatParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired_ttl_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_last_heartbeat: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkUpdateParams {
    /// Metadata patch. String values upsert; `null` values delete.
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkStopParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SelfHostedWork {
    pub id: String,
    pub environment_id: String,
    pub state: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub acknowledged_at: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub stopped_at: Option<String>,
    #[serde(default)]
    pub stop_requested_at: Option<String>,
    #[serde(default)]
    pub latest_heartbeat_at: Option<String>,
}

impl SelfHostedWork {
    pub fn session_id(&self) -> Option<&str> {
        self.data
            .get("type")
            .and_then(|v| v.as_str())
            .filter(|ty| *ty == "session")?;
        self.data.get("id").and_then(|v| v.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkHeartbeatResponse {
    pub last_heartbeat: String,
    pub lease_extended: bool,
    pub state: String,
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkQueueStats {
    pub depth: u64,
    pub pending: u64,
    pub workers_polling: u64,
    #[serde(default)]
    pub oldest_queued_at: Option<String>,
}

pub struct EnvironmentWorkService {
    client: Client,
}

impl EnvironmentWorkService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn get(&self, environment_id: &str, work_id: &str) -> Result<SelfHostedWork> {
        let path = format!("/v1/environments/{environment_id}/work/{work_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update(
        &self,
        environment_id: &str,
        work_id: &str,
        params: WorkUpdateParams,
    ) -> Result<SelfHostedWork> {
        let path = format!("/v1/environments/{environment_id}/work/{work_id}?beta=true");
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

    pub async fn list(
        &self,
        environment_id: &str,
        params: &WorkListParams,
    ) -> Result<Page<SelfHostedWork>> {
        let path = format!("/v1/environments/{environment_id}/work?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn ack(&self, environment_id: &str, work_id: &str) -> Result<SelfHostedWork> {
        let path = format!("/v1/environments/{environment_id}/work/{work_id}/ack?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn heartbeat(
        &self,
        environment_id: &str,
        work_id: &str,
        params: &WorkHeartbeatParams,
    ) -> Result<WorkHeartbeatResponse> {
        let path = format!("/v1/environments/{environment_id}/work/{work_id}/heartbeat?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn poll(
        &self,
        environment_id: &str,
        params: &WorkPollParams,
        worker_id: Option<&str>,
    ) -> Result<Option<SelfHostedWork>> {
        let path = format!("/v1/environments/{environment_id}/work/poll?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                let mut req = self
                    .client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
                    .query(params);
                if let Some(worker_id) = worker_id.filter(|id| !id.is_empty()) {
                    req = req.header("Anthropic-Worker-ID", worker_id);
                }
                req
            })
            .await?;
        json_or_none(resp).await
    }

    pub async fn stats(&self, environment_id: &str) -> Result<WorkQueueStats> {
        let path = format!("/v1/environments/{environment_id}/work/stats?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn stop(
        &self,
        environment_id: &str,
        work_id: &str,
        params: WorkStopParams,
    ) -> Result<SelfHostedWork> {
        let path = format!("/v1/environments/{environment_id}/work/{work_id}/stop?beta=true");
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
}

async fn json_or_none<T: for<'de> Deserialize<'de>>(resp: reqwest::Response) -> Result<Option<T>> {
    if resp.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }
    let body = resp.bytes().await?;
    if body.is_empty() || body.as_ref() == b"null" {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&body)?))
}
