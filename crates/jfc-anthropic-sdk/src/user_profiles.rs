//! `BetaUserProfileService` — multi-user enrollment.
//!
//! Endpoints (`anthropic-beta: user-profiles-2026-03-24`):
//! - `POST /v1/beta/user-profiles` — create
//! - `GET /v1/beta/user-profiles` — list
//! - `GET /v1/beta/user-profiles/{id}` — retrieve
//! - `PATCH /v1/beta/user-profiles/{id}` — update
//! - `POST /v1/beta/user-profiles/{id}/enrollment` — create enrollment URL

use crate::client::Client;
use serde::Deserialize;

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

pub struct UserProfileService {
    client: Client,
}

impl UserProfileService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}
