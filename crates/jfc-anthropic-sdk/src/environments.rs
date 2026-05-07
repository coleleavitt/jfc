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
    Cargo { name: String, version: Option<String> },
    Pip { name: String, version: Option<String> },
    Npm { name: String, version: Option<String> },
    Gem { name: String, version: Option<String> },
    Go { module: String, version: Option<String> },
    Apt { name: String, version: Option<String> },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Environment {
    pub id: String,
    pub name: String,
    pub created_at: String,
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
                    .request(Method::POST, "/v1/beta/environments", Some(beta::MANAGED_AGENTS))
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }
}
