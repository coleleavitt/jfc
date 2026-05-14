//! OpenWebUI auth-verification + instance-config helpers.
//!
//! Wraps the three "metadata" endpoints from `opencode-openwebui-auth/src/oauth/api.ts`:
//! - `GET /api/config`        — instance name/version/features (no auth required)
//! - `GET /api/v1/auths/`     — verify the JWT and return user identity
//! - `GET /api/models`        — list models accessible to the user

#![allow(dead_code)]

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct VerifiedUser {
    pub id: String,
    pub email: String,
    pub role: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    pub status: bool,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub features: std::collections::HashMap<String, serde_json::Value>,
}

/// Strip a trailing slash from `base_url` so callers can write
/// `format!("{base}/api/...")` safely.
pub fn normalize_base_url(base_url: &str) -> anyhow::Result<String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        anyhow::bail!("Base URL must start with http:// or https:// (got {trimmed})");
    }
    Ok(trimmed.to_owned())
}

/// `GET /api/config` — instance metadata. No auth required.
pub async fn fetch_instance_config(client: &reqwest::Client, base_url: &str) -> anyhow::Result<InstanceConfig> {
    let res = client
        .get(format!("{base_url}/api/config"))
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await?;
    if !res.status().is_success() {
        anyhow::bail!("GET /api/config failed: {}", res.status());
    }
    Ok(res.json().await?)
}

/// `GET /api/v1/auths/` — verify the JWT and return user identity.
pub async fn verify_token(client: &reqwest::Client, base_url: &str, token: &str) -> anyhow::Result<VerifiedUser> {
    let res = client
        .get(format!("{base_url}/api/v1/auths/"))
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        anyhow::bail!(
            "Token rejected ({status}): {}",
            &body[..body.len().min(200)]
        );
    }
    Ok(res.json().await?)
}
