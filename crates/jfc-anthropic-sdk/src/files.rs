//! `BetaFileService` — upload, list, download, delete, metadata.
//!
//! Endpoints (`anthropic-beta: files-api-2025-04-14`):
//! - `POST /v1/files?beta=true` — upload (multipart)
//! - `GET /v1/files?beta=true` — list
//! - `GET /v1/files/{id}?beta=true` — metadata
//! - `GET /v1/files/{id}/content?beta=true` — download
//! - `DELETE /v1/files/{id}?beta=true` — delete

use crate::beta;
use crate::client::Client;
use crate::error::{Error, Result};
use crate::pagination::{ListParams, Page};
use reqwest::Method;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct FileMetadata {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub created_at: String,
}

pub struct FileService {
    client: Client,
}

impl FileService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn list(&self) -> Result<Page<FileMetadata>> {
        self.list_page(&ListParams::default()).await
    }

    pub async fn list_page(&self, params: &ListParams) -> Result<Page<FileMetadata>> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, "/v1/files?beta=true", Some(beta::FILES))
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn metadata(&self, file_id: &str) -> Result<FileMetadata> {
        let path = format!("/v1/files/{file_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| self.client.request(Method::GET, &path, Some(beta::FILES)))
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete(&self, file_id: &str) -> Result<()> {
        let path = format!("/v1/files/{file_id}?beta=true");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::FILES))
            })
            .await?;
        Ok(())
    }

    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let path = format!("/v1/files/{file_id}/content?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| self.client.request(Method::GET, &path, Some(beta::FILES)))
            .await?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// Upload a file via multipart. Returns the new file's metadata
    /// (including the assigned `id`). Useful for swapping inline base64
    /// attachments with FileID references in long-running sessions.
    pub async fn upload(
        &self,
        filename: &str,
        mime_type: &str,
        bytes: Vec<u8>,
    ) -> Result<FileMetadata> {
        let url = format!("{}/v1/files?beta=true", self.client.base_url());
        let part = Part::bytes(bytes)
            .file_name(filename.to_owned())
            .mime_str(mime_type)
            .map_err(|e| Error::Other(format!("multipart mime: {e}")))?;
        let form = Form::new().part("file", part);
        let resp = self
            .client
            .request_url(Method::POST, url, Some(beta::FILES))
            .multipart(form)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(crate::client::into_api_error(resp).await);
        }
        Ok(resp.json().await?)
    }
}
