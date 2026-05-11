//! `BetaModelService` — capability discovery.
//!
//! Endpoints:
//! - `GET /v1/models` — list available models
//! - `GET /v1/models/{id}` — retrieve a single model

use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub id: String,
    pub display_name: String,
    pub created_at: Option<String>,
    pub max_tokens: Option<u32>,
    pub context_window: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelList {
    pub data: Vec<Model>,
    pub has_more: Option<bool>,
}

pub struct ModelService {
    client: Client,
}

impl ModelService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn list(&self) -> Result<ModelList> {
        let resp = self
            .client
            .execute_with_retry(|| self.client.request(Method::GET, "/v1/models", None))
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, model_id: &str) -> Result<Model> {
        let path = format!("/v1/models/{model_id}");
        let resp = self
            .client
            .execute_with_retry(|| self.client.request(Method::GET, &path, None))
            .await?;
        Ok(resp.json().await?)
    }
}
