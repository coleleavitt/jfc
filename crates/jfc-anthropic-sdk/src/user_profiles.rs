//! `BetaUserProfileService` — multi-user enrollment.
//!
//! Endpoints (`anthropic-beta: user-profiles-2026-03-24`):
//! - `POST /v1/user_profiles?beta=true` — create
//! - `GET /v1/user_profiles?beta=true` — list
//! - `GET /v1/user_profiles/{id}?beta=true` — retrieve
//! - `POST /v1/user_profiles/{id}?beta=true` — update
//! - `POST /v1/user_profiles/{id}/enrollment_url?beta=true` — create enrollment URL

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use crate::pagination::{ListParams, Page};
use reqwest::Method;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize)]
pub struct UserProfileCreateParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserProfile {
    pub id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnrollmentUrl {
    pub url: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UserProfileUpdateParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

pub struct UserProfileService {
    client: Client,
}

impl UserProfileService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn create(&self, params: UserProfileCreateParams) -> Result<UserProfile> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(
                        Method::POST,
                        "/v1/user_profiles?beta=true",
                        Some(beta::USER_PROFILES),
                    )
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn list(&self) -> Result<Page<UserProfile>> {
        self.list_page(&ListParams::default()).await
    }

    pub async fn list_page(&self, params: &ListParams) -> Result<Page<UserProfile>> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(
                        Method::GET,
                        "/v1/user_profiles?beta=true",
                        Some(beta::USER_PROFILES),
                    )
                    .query(params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn get(&self, user_profile_id: &str) -> Result<UserProfile> {
        let path = format!("/v1/user_profiles/{user_profile_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, &path, Some(beta::USER_PROFILES))
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn update(
        &self,
        user_profile_id: &str,
        params: UserProfileUpdateParams,
    ) -> Result<UserProfile> {
        let path = format!("/v1/user_profiles/{user_profile_id}?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::USER_PROFILES))
                    .json(&params)
            })
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn create_enrollment_url(&self, user_profile_id: &str) -> Result<EnrollmentUrl> {
        let path = format!("/v1/user_profiles/{user_profile_id}/enrollment_url?beta=true");
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::POST, &path, Some(beta::USER_PROFILES))
            })
            .await?;
        Ok(resp.json().await?)
    }
}
