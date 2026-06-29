//! `BetaVaultService` + `BetaVaultCredentialService` — secret storage.
//!
//! Endpoints (`anthropic-beta: managed-agents-2026-04-01`):
//! - `POST /v1/vaults?beta=true` — create vault
//! - `GET /v1/vaults?beta=true` — list
//! - `GET /v1/vaults/{id}?beta=true` — retrieve
//! - `POST /v1/vaults/{id}?beta=true` — update
//! - `DELETE /v1/vaults/{id}?beta=true` — delete
//! - `POST /v1/vaults/{id}/archive?beta=true` — archive
//! - `POST /v1/vaults/{id}/credentials?beta=true` — add credential
//! - `GET /v1/vaults/{id}/credentials?beta=true` — list credentials
//! - `POST /v1/vaults/{id}/credentials/{cid}/mcp_oauth_validate?beta=true` — validate

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use crate::pagination::{ListParams, Page};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct VaultCreateParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct VaultUpdateParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Vault {
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

/// Mirrors `BetaManagedAgentsCredentialAuthUnion`. Each variant covers
/// one auth scheme exposed by the SDK.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialAuth {
    StaticBearer {
        token: String,
        mcp_server_url: String,
    },
    McpOauth {
        mcp_server_url: String,
        access_token: String,
        expires_at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        refresh: Option<McpOauthRefresh>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOauthRefresh {
    pub client_id: String,
    pub refresh_token: String,
    pub token_endpoint: String,
    pub token_endpoint_auth: TokenEndpointAuth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenEndpointAuth {
    None,
    ClientSecretBasic { client_secret: String },
    ClientSecretPost { client_secret: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Credential {
    pub id: String,
    pub vault_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub auth: Option<CredentialAuth>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialCreateParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub auth: CredentialAuth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CredentialUpdateParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<CredentialAuth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CredentialValidation {
    pub credential_id: String,
    pub vault_id: String,
    pub status: String,
    #[serde(default)]
    pub has_refresh_token: bool,
    #[serde(default)]
    pub mcp_probe: serde_json::Value,
    #[serde(default)]
    pub refresh: serde_json::Value,
    #[serde(default)]
    pub validated_at: Option<String>,
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
                    .request(
                        Method::POST,
                        "/v1/vaults?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
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
                    .request(
                        Method::GET,
                        "/v1/vaults?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, vault_id: &str) -> Result<Vault> {
        let path = format!("/v1/vaults/{vault_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update(&self, vault_id: &str, params: VaultUpdateParams) -> Result<Vault> {
        let path = format!("/v1/vaults/{vault_id}?beta=true");
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

    pub async fn delete(&self, vault_id: &str) -> Result<()> {
        let path = format!("/v1/vaults/{vault_id}?beta=true");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }

    pub async fn archive(&self, vault_id: &str) -> Result<Vault> {
        let path = format!("/v1/vaults/{vault_id}/archive?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn add_credential(
        &self,
        vault_id: &str,
        params: CredentialCreateParams,
    ) -> Result<Credential> {
        let path = format!("/v1/vaults/{vault_id}/credentials?beta=true");
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

    pub async fn get_credential(&self, vault_id: &str, credential_id: &str) -> Result<Credential> {
        let path = format!("/v1/vaults/{vault_id}/credentials/{credential_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update_credential(
        &self,
        vault_id: &str,
        credential_id: &str,
        params: CredentialUpdateParams,
    ) -> Result<Credential> {
        let path = format!("/v1/vaults/{vault_id}/credentials/{credential_id}?beta=true");
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
        let path = format!("/v1/vaults/{vault_id}/credentials?beta=true");
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
        let path = format!("/v1/vaults/{vault_id}/credentials/{credential_id}?beta=true");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }

    pub async fn archive_credential(
        &self,
        vault_id: &str,
        credential_id: &str,
    ) -> Result<Credential> {
        let path = format!("/v1/vaults/{vault_id}/credentials/{credential_id}/archive?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn validate_mcp_oauth(
        &self,
        vault_id: &str,
        credential_id: &str,
    ) -> Result<CredentialValidation> {
        let path = format!(
            "/v1/vaults/{vault_id}/credentials/{credential_id}/mcp_oauth_validate?beta=true"
        );
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }
}
