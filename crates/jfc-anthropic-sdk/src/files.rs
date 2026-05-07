//! `BetaFileService` — upload, list, download, delete, metadata.
//!
//! Endpoints (`anthropic-beta: files-api-2025-04-14`):
//! - `POST /v1/beta/files` — upload (multipart)
//! - `GET /v1/beta/files` — list
//! - `GET /v1/beta/files/{id}` — metadata
//! - `GET /v1/beta/files/{id}/content` — download
//! - `DELETE /v1/beta/files/{id}` — delete

use crate::beta;
use crate::client::Client;
use crate::error::{Error, Result};
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

    pub async fn metadata(&self, file_id: &str) -> Result<FileMetadata> {
        let path = format!("/v1/beta/files/{file_id}");
        let resp = self
            .client
            .execute_with_retry(|| self.client.request(Method::GET, &path, Some(beta::FILES)))
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete(&self, file_id: &str) -> Result<()> {
        let path = format!("/v1/beta/files/{file_id}");
        self.client
            .execute_with_retry(|| self.client.request(Method::DELETE, &path, Some(beta::FILES)))
            .await?;
        Ok(())
    }

    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let path = format!("/v1/beta/files/{file_id}/content");
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
        let url = format!("{}/v1/beta/files", self.client.base_url());
        let part = Part::bytes(bytes)
            .file_name(filename.to_owned())
            .mime_str(mime_type)
            .map_err(|e| Error::Other(format!("multipart mime: {e}")))?;
        let form = Form::new().part("file", part);
        let resp = self
            .client
            .http()
            .request(Method::POST, url)
            .header("anthropic-beta", beta::FILES)
            .multipart(form)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Api {
                status,
                message: body,
                request_id: None,
                body: None,
            });
        }
        Ok(resp.json().await?)
    }
}
