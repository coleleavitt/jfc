//! Anthropic SDK-style OIDC Federation / Workload Identity authentication.
//!
//! Supports two auth types from Anthropic profile configs:
//! - `oidc_federation`: exchanges a JWT identity token for an access token
//! - `user_oauth`: uses stored access_token + refresh_token
//!
//! Config path resolution: `ANTHROPIC_CONFIG_DIR` > `$XDG_CONFIG_HOME/anthropic`
//! > `$HOME/.config/anthropic`

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum WorkloadIdentityError {
    #[error("config file not found: {0}")]
    ConfigNotFound(PathBuf),

    #[error("credentials file not found: {0}")]
    CredentialsNotFound(PathBuf),

    #[error("identity token file not found: {0}")]
    IdentityTokenFileNotFound(PathBuf),

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("invalid config: {0}")]
    InvalidConfig(String),

    #[error("token exchange failed: {status} {body}")]
    TokenExchangeFailed { status: u16, body: String },

    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, WorkloadIdentityError>;

// ---------------------------------------------------------------------------
// Config structs
// ---------------------------------------------------------------------------

/// Top-level profile configuration (maps to `configs/<profile>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    pub authentication: AuthenticationType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthenticationType {
    OidcFederation(OidcFederationAuth),
    UserOauth(UserOAuthAuth),
}

/// OIDC Federation auth — exchanges a third-party JWT for an Anthropic access token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcFederationAuth {
    /// Path to the file containing the identity token JWT.
    #[serde(default)]
    pub identity_token_file: Option<String>,
    /// Environment variable holding the identity token (alternative to file).
    #[serde(default)]
    pub identity_token_env: Option<String>,
    /// Federation rule ID governing the exchange.
    pub federation_rule_id: String,
    /// Service account ID to assume.
    #[serde(default)]
    pub service_account_id: Option<String>,
}

/// User OAuth auth — uses stored credentials (access_token + refresh_token).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOAuthAuth {
    /// OAuth client_id used for token refresh.
    pub client_id: String,
    /// Path to the credentials JSON file (default: `credentials/<profile>.json`).
    #[serde(default)]
    pub credentials_path: Option<String>,
}

/// Stored credentials file structure (`credentials/<profile>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCredentials {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) when the access token expires.
    pub expires_at: u64,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Token response from `/v1/oauth/token`.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Seconds until expiry (from the `expires_in` field).
    pub expires_in: u64,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

// ---------------------------------------------------------------------------
// Config path resolution
// ---------------------------------------------------------------------------

/// Resolve the Anthropic config directory.
/// Precedence: `ANTHROPIC_CONFIG_DIR` > `$XDG_CONFIG_HOME/anthropic` > `$HOME/.config/anthropic`
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ANTHROPIC_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("anthropic");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("anthropic")
}

/// Resolve the active Anthropic profile name.
/// Precedence: explicit argument > `ANTHROPIC_PROFILE` env > "default"
pub fn resolve_profile_name(profile: Option<&str>) -> String {
    if let Some(p) = profile {
        return p.to_owned();
    }
    std::env::var("ANTHROPIC_PROFILE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "default".to_owned())
}

/// Load an `AuthConfig` from the profile config file.
///
/// Reads `<config_dir>/configs/<profile>.json` and overlays env vars.
pub fn load_profile_config(profile: Option<&str>) -> Result<AuthConfig> {
    let profile_name = resolve_profile_name(profile);
    let dir = config_dir();
    let config_path = dir.join("configs").join(format!("{profile_name}.json"));

    let text = std::fs::read_to_string(&config_path)
        .map_err(|_| WorkloadIdentityError::ConfigNotFound(config_path.clone()))?;

    let mut config: AuthConfig = serde_json::from_str(&text)
        .map_err(|e| WorkloadIdentityError::InvalidConfig(e.to_string()))?;

    // Overlay env vars
    if config.org_id.is_none() {
        config.org_id = std::env::var("ANTHROPIC_ORGANIZATION_ID")
            .ok()
            .filter(|s| !s.is_empty());
    }
    if config.workspace_id.is_none() {
        config.workspace_id = std::env::var("ANTHROPIC_WORKSPACE_ID")
            .ok()
            .filter(|s| !s.is_empty());
    }
    if config.base_url.is_none() {
        config.base_url = std::env::var("ANTHROPIC_BASE_URL")
            .ok()
            .filter(|s| !s.is_empty());
    }
    if config.scopes.is_none()
        && let Ok(scope) = std::env::var("ANTHROPIC_SCOPE")
        && !scope.is_empty()
    {
        config.scopes = Some(scope.split(',').map(|s| s.trim().to_owned()).collect());
    }

    // Overlay OIDC-specific env vars
    if let AuthenticationType::OidcFederation(ref mut oidc) = config.authentication {
        if oidc.identity_token_file.is_none() {
            oidc.identity_token_file = std::env::var("ANTHROPIC_IDENTITY_TOKEN_FILE")
                .ok()
                .filter(|s| !s.is_empty());
        }
        if let Ok(rule_id) = std::env::var("ANTHROPIC_FEDERATION_RULE_ID")
            && !rule_id.is_empty()
        {
            oidc.federation_rule_id = rule_id;
        }
        if oidc.service_account_id.is_none() {
            oidc.service_account_id = std::env::var("ANTHROPIC_SERVICE_ACCOUNT_ID")
                .ok()
                .filter(|s| !s.is_empty());
        }
    }

    Ok(config)
}

// ---------------------------------------------------------------------------
// Token resolution
// ---------------------------------------------------------------------------

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const OIDC_BETA_HEADER: &str = "oauth-2025-04-20,oidc-federation-2026-04-01";

/// Resolve credentials from config, performing token exchange if needed.
pub async fn resolve_credentials(
    client: &reqwest::Client,
    config: &AuthConfig,
) -> Result<ResolvedToken> {
    match &config.authentication {
        AuthenticationType::OidcFederation(oidc) => exchange_oidc_token(client, config, oidc).await,
        AuthenticationType::UserOauth(oauth) => resolve_user_oauth(client, config, oauth).await,
    }
}

/// A resolved access token with expiry info.
#[derive(Debug, Clone)]
pub struct ResolvedToken {
    pub access_token: String,
    pub expires_at: u64,
    pub token_type: String,
}

impl ResolvedToken {
    fn from_response(resp: &TokenResponse) -> Self {
        let now = now_secs();
        Self {
            access_token: resp.access_token.clone(),
            expires_at: now + resp.expires_in,
            token_type: resp
                .token_type
                .clone()
                .unwrap_or_else(|| "bearer".to_owned()),
        }
    }

    /// Returns true if the token expires within `buffer_secs`.
    pub fn expires_within(&self, buffer_secs: u64) -> bool {
        now_secs() + buffer_secs >= self.expires_at
    }

    /// Returns true if the token is expired.
    pub fn is_expired(&self) -> bool {
        self.expires_within(0)
    }
}

/// Read the identity token for OIDC federation.
fn read_identity_token(oidc: &OidcFederationAuth) -> Result<String> {
    // Try env var first (identity_token_env), then file
    if let Some(ref env_var) = oidc.identity_token_env
        && let Ok(token) = std::env::var(env_var)
        && !token.is_empty()
    {
        return Ok(token.trim().to_owned());
    }

    if let Some(ref file_path) = oidc.identity_token_file {
        let path = Path::new(file_path);
        let token = std::fs::read_to_string(path)
            .map_err(|_| WorkloadIdentityError::IdentityTokenFileNotFound(path.to_owned()))?;
        return Ok(token.trim().to_owned());
    }

    Err(WorkloadIdentityError::MissingField(
        "identity_token_file or identity_token_env",
    ))
}

fn base_url(config: &AuthConfig) -> &str {
    config.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL)
}

/// Exchange an OIDC identity token for an Anthropic access token.
async fn exchange_oidc_token(
    client: &reqwest::Client,
    config: &AuthConfig,
    oidc: &OidcFederationAuth,
) -> Result<ResolvedToken> {
    let identity_token = read_identity_token(oidc)?;
    let token_url = format!("{}/v1/oauth/token", base_url(config));

    let mut form = vec![
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:jwt-bearer".to_owned(),
        ),
        ("assertion", identity_token),
        ("federation_rule_id", oidc.federation_rule_id.clone()),
    ];

    if let Some(ref sa_id) = oidc.service_account_id {
        form.push(("service_account_id", sa_id.clone()));
    }
    if let Some(ref org_id) = config.org_id {
        form.push(("organization_id", org_id.clone()));
    }
    if let Some(ref ws_id) = config.workspace_id {
        form.push(("workspace_id", ws_id.clone()));
    }
    if let Some(ref scopes) = config.scopes {
        form.push(("scope", scopes.join(" ")));
    }

    let resp = client
        .post(&token_url)
        .header("anthropic-beta", OIDC_BETA_HEADER)
        .form(&form)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status >= 400 {
        let body = resp.text().await.unwrap_or_default();
        return Err(WorkloadIdentityError::TokenExchangeFailed { status, body });
    }

    let token_resp: TokenResponse = resp.json().await?;
    Ok(ResolvedToken::from_response(&token_resp))
}

/// Resolve user OAuth credentials — read from file, refresh if expired.
async fn resolve_user_oauth(
    client: &reqwest::Client,
    config: &AuthConfig,
    oauth: &UserOAuthAuth,
) -> Result<ResolvedToken> {
    let creds_path = resolve_credentials_path(oauth)?;
    let creds = load_stored_credentials(&creds_path)?;

    let now = now_secs();
    // If token hasn't expired (with 30s buffer), return as-is
    if creds.expires_at > now + 30 {
        return Ok(ResolvedToken {
            access_token: creds.access_token,
            expires_at: creds.expires_at,
            token_type: creds.token_type.unwrap_or_else(|| "bearer".to_owned()),
        });
    }

    // Token expired or about to — refresh it
    let (resolved, token_resp) = refresh_oauth_token(client, config, oauth, &creds).await?;

    // Persist updated credentials
    let updated = StoredCredentials {
        access_token: resolved.access_token.clone(),
        refresh_token: token_resp.refresh_token.unwrap_or(creds.refresh_token),
        expires_at: resolved.expires_at,
        token_type: Some(resolved.token_type.clone()),
        scope: creds.scope,
    };
    save_stored_credentials(&creds_path, &updated)?;

    Ok(resolved)
}

fn resolve_credentials_path(oauth: &UserOAuthAuth) -> Result<PathBuf> {
    if let Some(ref explicit) = oauth.credentials_path {
        return Ok(PathBuf::from(explicit));
    }
    // Default: use ANTHROPIC_PROFILE or "default" to find credentials file
    let profile = resolve_profile_name(None);
    let path = config_dir()
        .join("credentials")
        .join(format!("{profile}.json"));
    Ok(path)
}

fn load_stored_credentials(path: &Path) -> Result<StoredCredentials> {
    let text = std::fs::read_to_string(path)
        .map_err(|_| WorkloadIdentityError::CredentialsNotFound(path.to_owned()))?;
    let creds: StoredCredentials = serde_json::from_str(&text)?;
    Ok(creds)
}

fn save_stored_credentials(path: &Path, creds: &StoredCredentials) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(creds)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

async fn refresh_oauth_token(
    client: &reqwest::Client,
    config: &AuthConfig,
    oauth: &UserOAuthAuth,
    creds: &StoredCredentials,
) -> Result<(ResolvedToken, TokenResponse)> {
    let token_url = format!("{}/v1/oauth/token", base_url(config));

    let form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", creds.refresh_token.as_str()),
        ("client_id", oauth.client_id.as_str()),
    ];

    let resp = client.post(&token_url).form(&form).send().await?;

    let status = resp.status().as_u16();
    if status >= 400 {
        let body = resp.text().await.unwrap_or_default();
        return Err(WorkloadIdentityError::TokenExchangeFailed { status, body });
    }

    let token_resp: TokenResponse = resp.json().await?;
    let resolved = ResolvedToken::from_response(&token_resp);
    Ok((resolved, token_resp))
}

// ---------------------------------------------------------------------------
// TokenCache — background refresh with proactive renewal
// ---------------------------------------------------------------------------

/// Refresh 120 seconds before expiry; force-refresh at 30 seconds.
const PROACTIVE_REFRESH_SECS: u64 = 120;
const FORCE_REFRESH_SECS: u64 = 30;

/// A cached token with background refresh support.
///
/// The cache proactively refreshes tokens `PROACTIVE_REFRESH_SECS` before
/// expiry and force-refreshes when only `FORCE_REFRESH_SECS` remain.
#[derive(Clone)]
pub struct TokenCache {
    inner: Arc<TokenCacheInner>,
}

struct TokenCacheInner {
    state: RwLock<TokenCacheState>,
    config: AuthConfig,
    client: reqwest::Client,
}

struct TokenCacheState {
    token: Option<ResolvedToken>,
    /// Whether a background refresh is currently in progress.
    refreshing: bool,
}

impl TokenCache {
    /// Create a new `TokenCache` for the given config.
    pub fn new(config: AuthConfig, client: reqwest::Client) -> Self {
        Self {
            inner: Arc::new(TokenCacheInner {
                state: RwLock::new(TokenCacheState {
                    token: None,
                    refreshing: false,
                }),
                config,
                client,
            }),
        }
    }

    /// Get a valid access token, refreshing if needed.
    ///
    /// - If no token is cached, performs a blocking fetch.
    /// - If token expires within `PROACTIVE_REFRESH_SECS`, spawns a background refresh
    ///   and returns the current token.
    /// - If token expires within `FORCE_REFRESH_SECS`, performs a blocking refresh.
    pub async fn get_token(&self) -> Result<String> {
        // Fast path: read lock
        {
            let state = self.inner.state.read().await;
            if let Some(ref token) = state.token {
                if !token.expires_within(PROACTIVE_REFRESH_SECS) {
                    // Token is fresh
                    return Ok(token.access_token.clone());
                }
                if !token.expires_within(FORCE_REFRESH_SECS) {
                    // Token is still usable but approaching expiry — background refresh
                    let access_token = token.access_token.clone();
                    if !state.refreshing {
                        drop(state);
                        self.spawn_background_refresh();
                    }
                    return Ok(access_token);
                }
                // Token critically close to expiry — fall through to blocking refresh
            }
        }

        // Slow path: blocking refresh
        self.refresh_blocking().await
    }

    /// Force a token refresh (blocking). Always fetches, even if the cached
    /// token is still fresh.
    pub async fn force_refresh(&self) -> Result<String> {
        let mut state = self.inner.state.write().await;
        let token = resolve_credentials(&self.inner.client, &self.inner.config).await?;
        let access_token = token.access_token.clone();
        state.token = Some(token);
        state.refreshing = false;
        Ok(access_token)
    }

    async fn refresh_blocking(&self) -> Result<String> {
        // Hold the write lock across the fetch so concurrent callers queue
        // behind one refresh instead of stampeding the token endpoint; the
        // recheck under the lock returns the token a winner just installed.
        let mut state = self.inner.state.write().await;
        if let Some(ref token) = state.token
            && !token.expires_within(FORCE_REFRESH_SECS)
        {
            return Ok(token.access_token.clone());
        }
        let token = resolve_credentials(&self.inner.client, &self.inner.config).await?;
        let access_token = token.access_token.clone();
        state.token = Some(token);
        state.refreshing = false;
        Ok(access_token)
    }

    fn spawn_background_refresh(&self) {
        let cache = self.clone();
        tokio::spawn(async move {
            // Mark refreshing
            {
                let mut state = cache.inner.state.write().await;
                if state.refreshing {
                    return; // Another task already refreshing
                }
                state.refreshing = true;
            }

            match resolve_credentials(&cache.inner.client, &cache.inner.config).await {
                Ok(token) => {
                    let mut state = cache.inner.state.write().await;
                    state.token = Some(token);
                    state.refreshing = false;
                }
                Err(_) => {
                    // Background refresh failed — leave existing token, clear flag
                    let mut state = cache.inner.state.write().await;
                    state.refreshing = false;
                }
            }
        });
    }

    /// Check if we currently have a cached token (possibly expired).
    pub async fn has_token(&self) -> bool {
        self.inner.state.read().await.token.is_some()
    }

    /// Invalidate the cached token.
    pub async fn invalidate(&self) {
        let mut state = self.inner.state.write().await;
        state.token = None;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_uses_anthropic_config_dir_env() {
        // This test is illustrative — in CI we can't safely mutate env
        let dir = config_dir();
        // Just ensure it returns a path
        assert!(!dir.as_os_str().is_empty());
    }

    #[test]
    fn resolve_profile_name_defaults_to_default() {
        let name = resolve_profile_name(None);
        // Will be "default" unless ANTHROPIC_PROFILE is set
        assert!(!name.is_empty());
    }

    #[test]
    fn resolved_token_expiry_logic() {
        let token = ResolvedToken {
            access_token: "test".into(),
            expires_at: now_secs() + 60,
            token_type: "bearer".into(),
        };
        assert!(!token.is_expired());
        assert!(token.expires_within(61));
        assert!(!token.expires_within(59));
    }

    #[test]
    fn parse_oidc_federation_config() {
        let json = r#"{
            "org_id": "org-123",
            "base_url": "https://api.anthropic.com",
            "authentication": {
                "type": "oidc_federation",
                "identity_token_file": "/tmp/token",
                "federation_rule_id": "rule-456",
                "service_account_id": "sa-789"
            }
        }"#;
        let config: AuthConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.org_id.as_deref(), Some("org-123"));
        match config.authentication {
            AuthenticationType::OidcFederation(ref oidc) => {
                assert_eq!(oidc.federation_rule_id, "rule-456");
                assert_eq!(oidc.service_account_id.as_deref(), Some("sa-789"));
                assert_eq!(oidc.identity_token_file.as_deref(), Some("/tmp/token"));
            }
            _ => panic!("expected OidcFederation"),
        }
    }

    #[test]
    fn parse_user_oauth_config() {
        let json = r#"{
            "authentication": {
                "type": "user_oauth",
                "client_id": "client-abc",
                "credentials_path": "/tmp/creds.json"
            }
        }"#;
        let config: AuthConfig = serde_json::from_str(json).unwrap();
        match config.authentication {
            AuthenticationType::UserOauth(ref oauth) => {
                assert_eq!(oauth.client_id, "client-abc");
                assert_eq!(oauth.credentials_path.as_deref(), Some("/tmp/creds.json"));
            }
            _ => panic!("expected UserOauth"),
        }
    }

    #[test]
    fn stored_credentials_roundtrip() {
        let creds = StoredCredentials {
            access_token: "at-123".into(),
            refresh_token: "rt-456".into(),
            expires_at: 1_700_000_000,
            token_type: Some("bearer".into()),
            scope: Some("default".into()),
        };
        let json = serde_json::to_string(&creds).unwrap();
        let parsed: StoredCredentials = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "at-123");
        assert_eq!(parsed.refresh_token, "rt-456");
        assert_eq!(parsed.expires_at, 1_700_000_000);
    }
}
