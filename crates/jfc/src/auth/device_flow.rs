//! OAuth 2.0 Device Authorization Grant (RFC 8628).
//!
//! Implements the device flow for environments where a browser redirect
//! isn't practical (SSH sessions, containers, headless CI). The user
//! visits a URL, enters a code, and the CLI polls until approved.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// URL-encode form parameters for POST body.
fn form_encode(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Configuration for the device flow endpoint.
#[derive(Debug, Clone)]
pub struct DeviceFlowConfig {
    /// OAuth client ID
    pub client_id: String,
    /// Device authorization endpoint (POST)
    pub device_auth_url: String,
    /// Token endpoint (POST) for polling
    pub token_url: String,
    /// Scopes to request
    pub scopes: Vec<String>,
}

/// Response from the device authorization endpoint.
#[derive(Debug, Deserialize)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_interval() -> u64 {
    5
}

/// Token response after successful authorization.
#[derive(Debug, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Error response during polling.
#[derive(Debug, Deserialize)]
struct PollErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Errors that can occur during the device flow.
#[derive(Debug, thiserror::Error)]
pub enum DeviceFlowError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Device code expired — user did not approve in time")]
    Expired,
    #[error("Authorization denied by user")]
    AccessDenied,
    #[error("Unexpected error from token endpoint: {0}")]
    TokenError(String),
}

/// Initiate the device authorization flow.
/// Returns the device auth response containing the user code and verification URL.
pub async fn request_device_code(
    client: &reqwest::Client,
    config: &DeviceFlowConfig,
) -> Result<DeviceAuthResponse, DeviceFlowError> {
    let mut params = vec![("client_id", config.client_id.as_str())];
    let scope_str = config.scopes.join(" ");
    if !scope_str.is_empty() {
        params.push(("scope", &scope_str));
    }

    let body = form_encode(&params);
    let resp = client
        .post(&config.device_auth_url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?
        .json::<DeviceAuthResponse>()
        .await?;

    Ok(resp)
}

/// Poll the token endpoint until the user approves or the code expires.
/// Returns the token response on success.
pub async fn poll_for_token(
    client: &reqwest::Client,
    config: &DeviceFlowConfig,
    device_code: &str,
    interval: u64,
    expires_in: u64,
) -> Result<TokenResponse, DeviceFlowError> {
    let poll_interval = Duration::from_secs(interval);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);

    loop {
        tokio::time::sleep(poll_interval).await;

        if tokio::time::Instant::now() >= deadline {
            return Err(DeviceFlowError::Expired);
        }

        let form_body = form_encode(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code),
            ("client_id", &config.client_id),
        ]);
        let resp = client
            .post(&config.token_url)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form_body)
            .send()
            .await?;

        let status = resp.status();
        let body_bytes = resp.bytes().await?;
        let body = String::from_utf8_lossy(&body_bytes);

        if status.is_success()
            && let Ok(token) = serde_json::from_str::<TokenResponse>(&body)
        {
            return Ok(token);
        }

        // Check for expected polling errors
        if let Ok(err) = serde_json::from_str::<PollErrorResponse>(&body) {
            match err.error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    // Back off by adding 5 seconds
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
                "access_denied" => {
                    return Err(match err.error_description {
                        Some(description) => DeviceFlowError::TokenError(description),
                        None => DeviceFlowError::AccessDenied,
                    });
                }
                "expired_token" => {
                    return Err(match err.error_description {
                        Some(description) => DeviceFlowError::TokenError(description),
                        None => DeviceFlowError::Expired,
                    });
                }
                other => {
                    return Err(DeviceFlowError::TokenError(
                        err.error_description.unwrap_or_else(|| other.to_string()),
                    ));
                }
            }
        }
    }
}

/// Store a token to `.jfc/credentials.json`.
pub fn store_token(token: &TokenResponse) -> std::io::Result<()> {
    let path = credentials_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(token).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    // Restrict permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Load a previously stored token from `.jfc/credentials.json`.
pub fn load_token() -> Option<TokenResponse> {
    let path = credentials_path();
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn credentials_path() -> PathBuf {
    PathBuf::from(".jfc/credentials.json")
}
