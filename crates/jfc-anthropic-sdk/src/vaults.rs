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

use crate::client::Client;
use serde::{Deserialize, Serialize};

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
    Bearer { token: String },
    GithubPat { token: String },
    OauthTokenPair { access_token: String, refresh_token: Option<String>, expires_at: Option<String> },
    McpOauth { client_id: String, access_token: String, refresh_token: Option<String> },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Credential {
    pub id: String,
    pub vault_id: String,
    pub display_name: String,
    pub created_at: String,
}

pub struct VaultService {
    client: Client,
}

impl VaultService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}
