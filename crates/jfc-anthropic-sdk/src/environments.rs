//! `BetaEnvironmentService` — isolated execution contexts.
//!
//! Endpoints (`anthropic-beta: managed-agents-2026-04-01`):
//! - `POST /v1/beta/environments` — create
//! - `GET /v1/beta/environments` — list
//! - `GET /v1/beta/environments/{id}` — retrieve
//! - `PATCH /v1/beta/environments/{id}` — update
//! - `DELETE /v1/beta/environments/{id}` — delete

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use crate::pagination::{ListParams, Page};
use reqwest::Method;
use serde::{Deserialize, Serialize};

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
    pub created_at: String,
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
                        "/v1/beta/environments",
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
                        "/v1/beta/environments",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, environment_id: &str) -> Result<Environment> {
        let path = format!("/v1/beta/environments/{environment_id}");
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
        let path = format!("/v1/beta/environments/{environment_id}");
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

    pub async fn delete(&self, environment_id: &str) -> Result<()> {
        let path = format!("/v1/beta/environments/{environment_id}");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }
}
