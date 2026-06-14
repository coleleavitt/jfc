//! `MemoryStoreService` — memory stores, memories, and memory versions.
//!
//! Mirrors the upstream SDK paths:
//! - `/v1/memory_stores?beta=true`
//! - `/v1/memory_stores/{store}/memories?beta=true`
//! - `/v1/memory_stores/{store}/memory_versions?beta=true`

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use crate::pagination::{ListParams, Page};
use reqwest::Method;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStore {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryStoreCreate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryStoreUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryCreate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(flatten)]
    pub body: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(flatten)]
    pub body: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryVersion {
    pub id: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

pub struct MemoryStoreService {
    client: Client,
}

impl MemoryStoreService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn create(&self, params: &MemoryStoreCreate) -> Result<MemoryStore> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(
                        Method::POST,
                        "/v1/memory_stores?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .json(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn retrieve(&self, store_id: &str) -> Result<MemoryStore> {
        let path = format!("/v1/memory_stores/{store_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update(&self, store_id: &str, params: &MemoryStoreUpdate) -> Result<MemoryStore> {
        let path = format!("/v1/memory_stores/{store_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
                    .json(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn list(&self, params: &ListParams) -> Result<Page<MemoryStore>> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(
                        Method::GET,
                        "/v1/memory_stores?beta=true",
                        Some(beta::MANAGED_AGENTS),
                    )
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete(&self, store_id: &str) -> Result<()> {
        let path = format!("/v1/memory_stores/{store_id}?beta=true");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(())
    }

    pub async fn archive(&self, store_id: &str) -> Result<MemoryStore> {
        let path = format!("/v1/memory_stores/{store_id}/archive?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn create_memory(&self, store_id: &str, params: &MemoryCreate) -> Result<Memory> {
        let path = format!("/v1/memory_stores/{store_id}/memories?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                let mut req = self
                    .client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS));
                if let Some(view) = params.view.as_deref() {
                    req = req.query(&[("view", view)]);
                }
                req.json(&params.body)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn retrieve_memory(&self, store_id: &str, memory_id: &str) -> Result<Memory> {
        let path = format!("/v1/memory_stores/{store_id}/memories/{memory_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update_memory(
        &self,
        store_id: &str,
        memory_id: &str,
        params: &MemoryUpdate,
    ) -> Result<Memory> {
        let path = format!("/v1/memory_stores/{store_id}/memories/{memory_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                let mut req = self
                    .client
                    .request(Method::POST, &path, Some(beta::MANAGED_AGENTS));
                if let Some(view) = params.view.as_deref() {
                    req = req.query(&[("view", view)]);
                }
                req.json(&params.body)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn list_memories(&self, store_id: &str, params: &ListParams) -> Result<Page<Memory>> {
        let path = format!("/v1/memory_stores/{store_id}/memories?beta=true");
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

    pub async fn delete_memory(
        &self,
        store_id: &str,
        memory_id: &str,
        expected_content_sha256: &str,
    ) -> Result<()> {
        let path = format!("/v1/memory_stores/{store_id}/memories/{memory_id}?beta=true");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::MANAGED_AGENTS))
                    .query(&[("expected_content_sha256", expected_content_sha256)])
            })
            .await?;
        Ok(())
    }

    pub async fn list_memory_versions(
        &self,
        store_id: &str,
        params: &ListParams,
    ) -> Result<Page<MemoryVersion>> {
        let path = format!("/v1/memory_stores/{store_id}/memory_versions?beta=true");
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

    pub async fn retrieve_memory_version(
        &self,
        store_id: &str,
        version_id: &str,
    ) -> Result<MemoryVersion> {
        let path = format!("/v1/memory_stores/{store_id}/memory_versions/{version_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MANAGED_AGENTS))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn redact_memory_version(
        &self,
        store_id: &str,
        version_id: &str,
    ) -> Result<MemoryVersion> {
        let path =
            format!("/v1/memory_stores/{store_id}/memory_versions/{version_id}/redact?beta=true");
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
