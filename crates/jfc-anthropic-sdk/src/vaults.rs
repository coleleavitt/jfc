//! `BetaVaultService` + `BetaVaultCredentialService` — secret storage.
//!
//! Endpoints (`anthropic-beta: managed-agents-2026-04-01`):
//! - `POST /v1/beta/vaults` — create vault
//! - `GET /v1/beta/vaults` — list
//! - `GET /v1/beta/vaults/{id}` — retrieve
//! - `DELETE /v1/beta/vaults/{id}` — delete
//! - `POST /v1/beta/vaults/{id}/credentials` — add credential
//! - `GET /v1/beta/vaults/{id}/credentials` — list credentials
//! - `DELETE /v1/beta/vaults/{id}/credentials/{cid}` — delete credential

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use crate::pagination::{ListParams, Page};
use reqwest::Method;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct VaultCreateParams {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Vault {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// Mirrors `BetaManagedAgentsCredentialAuthUnion`. Each variant covers
/// one auth scheme exposed by the SDK.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialAuth {
    Bearer {
        token: String,
    },
    GithubPat {
        token: String,
    },
    OauthTokenPair {
        access_token: String,
        refresh_token: Option<String>,
        expires_at: Option<String>,
    },
    McpOauth {
        client_id: String,
        access_token: String,
        refresh_token: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Credential {
    pub id: String,
    pub vault_id: String,
    pub display_name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialCreateParams {
    pub display_name: String,
    pub auth: CredentialAuth,
}

pub struct VaultService {
    client: Client,
}

impl VaultService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn create(&self, params: VaultCreateParams) -> Result<Vault> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, "/v1/beta/vaults", Some(beta::MANAGED_AGENTS))
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn list(&self) -> Result<Page<Vault>> {
        self.list_page(&ListParams::default()).await
    }

    pub async fn list_page(&self, params: &ListParams) -> Result<Page<Vault>> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, "/v1/beta/vaults", Some(beta::MANAGED_AGENTS))
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, vault_id: &str) -> Result<Vault> {
        let path = format!("/v1/beta/vaults/{vault_id}");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete(&self, vault_id: &str) -> Result<()> {
        let path = format!("/v1/beta/vaults/{vault_id}");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }

    pub async fn add_credential(
        &self,
        vault_id: &str,
        params: CredentialCreateParams,
    ) -> Result<Credential> {
        let path = format!("/v1/beta/vaults/{vault_id}/credentials");
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

    pub async fn list_credentials(&self, vault_id: &str) -> Result<Page<Credential>> {
        let path = format!("/v1/beta/vaults/{vault_id}/credentials");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete_credential(&self, vault_id: &str, credential_id: &str) -> Result<()> {
        let path = format!("/v1/beta/vaults/{vault_id}/credentials/{credential_id}");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }
}
