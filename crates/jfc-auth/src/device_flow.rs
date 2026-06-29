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
    let _linkscope_device = linkscope::phase("auth.device.request_code");
    linkscope::record_items("auth.device.request_code", 1);
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

    linkscope::record_items("auth.device.request_code.ok", 1);
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
    let _linkscope_poll = linkscope::phase("auth.device.poll_token");
    let poll_interval = Duration::from_secs(interval);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);

    loop {
        linkscope::record_items("auth.device.poll_attempt", 1);
        tokio::time::sleep(poll_interval).await;

        if tokio::time::Instant::now() >= deadline {
            linkscope::record_items("auth.device.poll_expired", 1);
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
            linkscope::record_items("auth.device.poll_ok", 1);
            return Ok(token);
        }

        // Check for expected polling errors
        if let Ok(err) = serde_json::from_str::<PollErrorResponse>(&body) {
            match err.error.as_str() {
                "authorization_pending" => {
                    linkscope::record_items("auth.device.poll_pending", 1);
                    continue;
                }
                "slow_down" => {
                    linkscope::record_items("auth.device.poll_slow_down", 1);
                    // Back off by adding 5 seconds
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
                "access_denied" => {
                    linkscope::record_items("auth.device.poll_denied", 1);
                    return Err(match err.error_description {
                        Some(description) => DeviceFlowError::TokenError(description),
                        None => DeviceFlowError::AccessDenied,
                    });
                }
                "expired_token" => {
                    linkscope::record_items("auth.device.poll_expired_token", 1);
                    return Err(match err.error_description {
                        Some(description) => DeviceFlowError::TokenError(description),
                        None => DeviceFlowError::Expired,
                    });
                }
                other => {
                    linkscope::record_items("auth.device.poll_token_error", 1);
                    return Err(DeviceFlowError::TokenError(
                        err.error_description.unwrap_or_else(|| other.to_string()),
                    ));
                }
            }
        }
    }
}

/// Store the device-flow token in the user-scoped credential store.
///
/// Reusable OAuth bearer/refresh tokens must never be written inside the active
/// repository: a shared or malicious checkout is an untrusted storage boundary
/// (CS-JFC-001). If a legacy repo-local `.jfc/credentials.json` exists it is
/// migrated (deleted) so a poisoned project file cannot shadow the real token.
pub fn store_token(token: &TokenResponse) -> std::io::Result<()> {
    let _linkscope_store = linkscope::phase("auth.device.store_token");
    write_token_file(token, &credentials_path())?;
    // Migrate away from the legacy repo-local store so stale or attacker-planted
    // credentials in a checkout cannot be picked up by `load_token`.
    let legacy = legacy_repo_credentials_path();
    if legacy.exists() {
        let _ = std::fs::remove_file(&legacy);
        linkscope::record_items("auth.device.legacy_token_removed", 1);
    }
    linkscope::record_items("auth.device.store_token.ok", 1);
    Ok(())
}

/// Write a token JSON file with `0o600` permissions on Unix. Path is explicit so
/// the store logic is unit-testable without touching the user config dir.
fn write_token_file(token: &TokenResponse, path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(token).map_err(std::io::Error::other)?;
    std::fs::write(path, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Load a previously stored token from the user-scoped store, falling back to a
/// legacy repo-local `.jfc/credentials.json` only for read-time migration.
pub fn load_token() -> Option<TokenResponse> {
    let _linkscope_load = linkscope::phase("auth.device.load_token");
    let user = credentials_path();
    if let Some(token) = std::fs::read_to_string(&user)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        linkscope::record_items("auth.device.load_token.user", 1);
        return Some(token);
    }
    let legacy = legacy_repo_credentials_path();
    let token = std::fs::read_to_string(&legacy)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    if token.is_some() {
        linkscope::record_items("auth.device.load_token.legacy", 1);
    } else {
        linkscope::record_items("auth.device.load_token.miss", 1);
    }
    token
}

/// Remove every device-flow credential store (user-scoped and legacy
/// repo-local). Returns the paths that were actually removed so `/logout` can
/// report exactly which credential files it cleared.
pub fn clear_token() -> Vec<PathBuf> {
    let _linkscope_clear = linkscope::phase("auth.device.clear_token");
    let removed = remove_existing_files(&[credentials_path(), legacy_repo_credentials_path()]);
    linkscope::record_items(
        "auth.device.clear_token.removed",
        usize_to_u64_saturating(removed.len()),
    );
    removed
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Remove each path that exists, returning the ones actually deleted. Path-driven
/// so the removal logic is unit-testable without touching the real user store.
fn remove_existing_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for path in paths {
        if path.exists() && std::fs::remove_file(path).is_ok() {
            removed.push(path.clone());
        }
    }
    removed
}

/// User-scoped device-flow credential path: `~/.config/jfc/credentials.json`.
pub fn credentials_path() -> PathBuf {
    dirs::config_dir()
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jfc")
        .join("credentials.json")
}

/// Legacy repo-local credential path retained only for migration/cleanup.
pub fn legacy_repo_credentials_path() -> PathBuf {
    PathBuf::from(".jfc").join("credentials.json")
}

#[cfg(test)]
mod credential_storage_tests {
    use super::*;

    fn sample_token() -> TokenResponse {
        TokenResponse {
            access_token: "secret-access".to_string(),
            token_type: "Bearer".to_string(),
            expires_in: Some(3600),
            refresh_token: Some("secret-refresh".to_string()),
            scope: None,
        }
    }

    // CS-JFC-001: reusable OAuth tokens must land in a user-scoped store, never
    // in the repo-relative `.jfc/credentials.json` sink.
    #[test]
    fn credentials_path_is_user_scoped_not_repo_local_regression() {
        let user = credentials_path();
        assert!(
            user.ends_with("jfc/credentials.json"),
            "expected user-scoped jfc/credentials.json, got {}",
            user.display()
        );
        assert_ne!(
            user,
            PathBuf::from(".jfc/credentials.json"),
            "device-flow store must not be the repo-local .jfc path"
        );
        assert_ne!(user, legacy_repo_credentials_path());
        // A user-scoped store has more than two path components
        // (e.g. ~/.config/jfc/credentials.json), unlike the 2-component repo path.
        assert!(user.components().count() > 2);
    }

    #[test]
    fn write_token_file_writes_json_with_locked_permissions_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jfc").join("credentials.json");
        write_token_file(&sample_token(), &path).unwrap();
        let loaded: TokenResponse =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.access_token, "secret-access");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    // CS-JFC-001/009: clearing must remove every device-flow store including the
    // legacy repo-local one, and report exactly what it removed.
    #[test]
    fn remove_existing_files_only_reports_deleted_robust() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("credentials.json");
        let absent = dir.path().join("missing.json");
        write_token_file(&sample_token(), &present).unwrap();

        let removed = remove_existing_files(&[present.clone(), absent.clone()]);

        assert_eq!(removed, vec![present.clone()]);
        assert!(!present.exists());
        assert!(!absent.exists());
    }
}
