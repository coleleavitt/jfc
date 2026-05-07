//! `BetaMessageBatchService` — async batch processing.
//!
//! Endpoints (`anthropic-beta: message-batches-2024-09-24`):
//! - `POST /v1/beta/messages/batches` — create
//! - `GET /v1/beta/messages/batches` — list
//! - `GET /v1/beta/messages/batches/{id}` — retrieve
//! - `POST /v1/beta/messages/batches/{id}/cancel` — cancel
//! - `DELETE /v1/beta/messages/batches/{id}` — delete
//! - `GET /v1/beta/messages/batches/{id}/results` — streamed results (JSONL)

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct MessageBatch {
    pub id: String,
    pub processing_status: BatchStatus,
    pub request_counts: BatchCounts,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    InProgress,
    Canceling,
    Ended,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BatchCounts {
    pub processing: u32,
    pub succeeded: u32,
    pub errored: u32,
    pub canceled: u32,
    pub expired: u32,
}

pub struct MessageBatchService {
    client: Client,
}

impl MessageBatchService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn get(&self, batch_id: &str) -> Result<MessageBatch> {
        let path = format!("/v1/beta/messages/batches/{batch_id}");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::MESSAGE_BATCHES))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn cancel(&self, batch_id: &str) -> Result<MessageBatch> {
        let path = format!("/v1/beta/messages/batches/{batch_id}/cancel");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::MESSAGE_BATCHES))
            })
            .await?;
        Ok(resp.json().await?)
    }

    /// Submit a batch of message requests. Each item in `requests` is a
    /// `(custom_id, MessageRequest)` pair so the caller can correlate
    /// results when they stream back. Returns the batch's `id` so
    /// callers can poll `get` / stream `results`.
    pub async fn create(
        &self,
        requests: Vec<BatchRequest>,
    ) -> Result<MessageBatch> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, "/v1/beta/messages/batches", Some(beta::MESSAGE_BATCHES))
                    .json(&serde_json::json!({ "requests": requests }))
            })
            .await?;
        Ok(resp.json().await?)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BatchRequest {
    pub custom_id: String,
    pub params: crate::messages::MessageRequest,
}
