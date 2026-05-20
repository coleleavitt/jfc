//! PKCE OAuth login flow for adding new Anthropic accounts.
//!
//! Port of `opencode-anthropic-auth/src/oauth/{pkce,tokens,api}.ts`.
//!
//! ## Flow
//!
//! 1. [`authorize`] generates a PKCE verifier + challenge + CSRF state, returns
//!    a URL the user opens in a browser.
//! 2. After Anthropic's web UI completes login, the callback page shows a
//!    `code#state` string. The user pastes it back into jfc.
//! 3. [`exchange_code`] swaps `code` for a token pair, validates the returned
//!    `state` (timing-safe), checks scopes, and returns the credentials.
//! 4. [`fetch_profile`] calls `/api/oauth/profile` to harvest tier / plan /
//!    organization metadata so [`AccountManager::pick_next`] can rank.
//! 5. [`login`] composes 3+4: it persists a fully-populated [`Account`] into
//!    the [`AccountManager`].
//!
//! All tokens are validated for shape (`sk-ant-oat01-…`) before being
//! persisted.

use std::time::Duration;

use base64::Engine;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::anthropic_accounts::{Account, AccountManager, now_ms};

const DEFAULT_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_AI_AUTHORIZE_URL: &str = "https://claude.com/cai/oauth/authorize";
const MANUAL_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const ROLES_URL: &str = "https://api.anthropic.com/api/oauth/claude_cli/roles";

/// Scopes for the initial authorize step. Mirrors opencode `AUTHORIZE_SCOPES`
/// in `oauth/tokens.ts`. `org:create_api_key` was deliberately removed by
/// upstream (CLI v89) — including it triggers `invalid_scope` on newer
/// accounts.
const AUTHORIZE_SCOPES: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

/// HTTP timeout for the token endpoint. Matches opencode's
/// `OAUTH_TIMEOUT_MS = 30000`.
const OAUTH_TIMEOUT: Duration = Duration::from_secs(30);
/// Profile-fetch timeout, intentionally shorter to avoid hanging logins.
const PROFILE_TIMEOUT: Duration = Duration::from_secs(10);

/// PKCE challenge bundle returned by [`authorize`]. The `verifier` MUST be
/// kept private (it's the secret half of the proof-of-possession protocol);
/// `state` MUST round-trip through the redirect to defeat CSRF.
#[derive(Debug, Clone)]
pub struct AuthorizeRequest {
    pub url: String,
    pub verifier: String,
    pub state: String,
    pub redirect_uri: String,
}

/// Successful token-exchange result. `expires_at_ms` is already adjusted for
/// the 30s skew opencode applies (`Date.now() + expires_in*1000 - 30000`).
#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: u64,
    #[allow(dead_code)]
    pub scopes: Vec<String>,
}

/// Profile data harvested from `/api/oauth/profile`, mapped to the fields
/// jfc actually consumes (tier ranking, display, org gating).
#[derive(Debug, Clone, Default)]
pub struct ProfileSnapshot {
    pub email: Option<String>,
    pub uuid: Option<String>,
    pub plan: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub full_name: Option<String>,
    pub organization_uuid: Option<String>,
    pub organization_name: Option<String>,
    pub organization_type: Option<String>,
    pub billing_type: Option<String>,
    pub display_name: Option<String>,
    pub has_claude_max: bool,
    pub has_claude_pro: bool,
    pub has_extra_usage_enabled: bool,
    pub subscription_status: Option<String>,
    pub organization_role: Option<String>,
    pub workspace_uuid: Option<String>,
    pub workspace_name: Option<String>,
    pub workspace_role: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RedirectTarget {
    Manual,
    Localhost(u16),
}

/// Errors surfaced by the login pipeline. `Permanent` means the credentials
/// or scopes are unrecoverable (don't retry); `Transient` means a network or
/// 5xx hiccup that's safe to retry.
#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("oauth: code/state pair was empty or malformed")]
    EmptyCode,
    #[error("oauth: CSRF state mismatch")]
    StateMismatch,
    #[error("oauth: missing scope user:inference in granted scopes")]
    MissingInferenceScope,
    #[error("oauth: returned token has unexpected shape")]
    BadTokenShape,
    #[error("oauth: permanent failure ({0})")]
    Permanent(String),
    #[error("oauth: transient failure ({0})")]
    Transient(String),
    #[error("oauth: io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("oauth: account error: {0}")]
    Account(#[from] anyhow::Error),
}

/// Generate a verifier+challenge+state bundle and the authorize URL.
///
/// The verifier is 32 random bytes, base64url-encoded (43 chars). The
/// challenge is `base64url(SHA-256(verifier))`. The state is a separate
/// 32-random-byte base64url string (independent from verifier per CLI
/// behavior).
pub fn authorize() -> AuthorizeRequest {
    authorize_with_redirect(RedirectTarget::Manual)
}

pub fn authorize_with_redirect(target: RedirectTarget) -> AuthorizeRequest {
    let verifier = generate_random_b64url(32);
    let challenge = sha256_b64url(&verifier);
    let state = generate_random_b64url(32);
    let redirect_uri = match target {
        RedirectTarget::Manual => MANUAL_REDIRECT_URI.to_owned(),
        RedirectTarget::Localhost(port) => format!("http://localhost:{port}/callback"),
    };
    let client_id = client_id();
    let url = format!(
        "{CLAUDE_AI_AUTHORIZE_URL}?code=true&client_id={cid}&response_type=code&redirect_uri={redir}\
         &scope={scope}&code_challenge={chal}&code_challenge_method=S256&state={st}",
        cid = url_encode(&client_id),
        redir = url_encode(&redirect_uri),
        scope = url_encode(AUTHORIZE_SCOPES),
        chal = url_encode(&challenge),
        st = url_encode(&state),
    );
    let mut url = url;
    if let Some(login_method) = force_login_method() {
        url.push_str("&login_method=");
        url.push_str(&url_encode(&login_method));
    }
    if let Some(org_uuid) = forced_org_uuid() {
        url.push_str("&orgUUID=");
        url.push_str(&url_encode(&org_uuid));
    }
    AuthorizeRequest {
        url,
        verifier,
        state,
        redirect_uri,
    }
}

/// Parse a `code#state` paste from the Anthropic callback page into its
/// constituent parts. Returns `(code, state)` — `state` is `None` if the
/// paste lacked the `#…` suffix.
fn split_callback_paste(raw: &str) -> (String, Option<String>) {
    if let Some(hash_idx) = raw.rfind('#') {
        let (code, rest) = raw.split_at(hash_idx);
        let state = rest.trim_start_matches('#').trim();
        let state = if state.is_empty() {
            None
        } else {
            Some(state.to_owned())
        };
        (code.trim().to_owned(), state)
    } else {
        (raw.trim().to_owned(), None)
    }
}

/// Exchange the authorization code for OAuth tokens.
///
/// `raw_code_state` is the entire `code#state` blob the user copied from
/// the callback page. The `state` returned by Anthropic must equal
/// `expected_state` (timing-safe compare) or this returns
/// [`LoginError::StateMismatch`].
#[allow(dead_code)]
pub async fn exchange_code(
    raw_code_state: &str,
    verifier: &str,
    expected_state: &str,
) -> Result<TokenPair, LoginError> {
    exchange_code_with_redirect(
        raw_code_state,
        verifier,
        expected_state,
        MANUAL_REDIRECT_URI,
    )
    .await
}

pub async fn exchange_code_with_redirect(
    raw_code_state: &str,
    verifier: &str,
    expected_state: &str,
    redirect_uri: &str,
) -> Result<TokenPair, LoginError> {
    let (code, returned_state) = split_callback_paste(raw_code_state);
    exchange_code_parts(
        &code,
        returned_state.as_deref(),
        verifier,
        expected_state,
        redirect_uri,
    )
    .await
}

pub async fn exchange_code_parts(
    code: &str,
    returned_state: Option<&str>,
    verifier: &str,
    expected_state: &str,
    redirect_uri: &str,
) -> Result<TokenPair, LoginError> {
    let code = code.trim();
    if code.is_empty() {
        return Err(LoginError::EmptyCode);
    }
    let returned_state = returned_state.ok_or(LoginError::StateMismatch)?;
    if !timing_safe_equal(returned_state.as_bytes(), expected_state.as_bytes()) {
        return Err(LoginError::StateMismatch);
    }

    let client = Client::builder()
        .timeout(OAUTH_TIMEOUT)
        .build()
        .map_err(|e| LoginError::Transient(format!("reqwest builder: {e}")))?;

    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": redirect_uri,
        "client_id": client_id(),
        "code_verifier": verifier,
        "state": expected_state,
    });

    let resp = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| LoginError::Transient(format!("token request: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| LoginError::Transient(format!("read body: {e}")))?;

    if !status.is_success() {
        let (oauth_error, detail) = parse_oauth_error(&text);
        let permanent_oauth = matches!(
            oauth_error.as_deref(),
            Some(
                "invalid_grant"
                    | "invalid_client"
                    | "invalid_request"
                    | "unauthorized_client"
                    | "access_denied"
                    | "unsupported_grant_type"
                    | "invalid_scope"
            )
        );
        let is_permanent = status.as_u16() == 401
            || status.as_u16() == 403
            || (status.as_u16() == 400 && permanent_oauth);
        return if is_permanent {
            Err(LoginError::Permanent(format!("status={status} {detail}")))
        } else {
            Err(LoginError::Transient(format!("status={status} {detail}")))
        };
    }

    let json: TokenResponse = serde_json::from_str(&text)
        .map_err(|e| LoginError::Transient(format!("parse token JSON: {e}")))?;

    let scopes: Vec<String> = json
        .scope
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if !scopes.iter().any(|s| s == "user:inference") {
        return Err(LoginError::MissingInferenceScope);
    }

    if !is_valid_access_token(&json.access_token) {
        return Err(LoginError::BadTokenShape);
    }
    if !is_valid_refresh_token(&json.refresh_token) {
        return Err(LoginError::BadTokenShape);
    }

    let expires_in = json.expires_in.unwrap_or(3600.0);
    if !expires_in.is_finite() || expires_in <= 0.0 {
        return Err(LoginError::Transient(format!(
            "invalid expires_in={expires_in}"
        )));
    }

    // Mirror opencode's 30s skew so we trigger refresh slightly early.
    let expires_at_ms = now_ms() + (expires_in * 1000.0) as u64 - 30_000;

    Ok(TokenPair {
        access_token: json.access_token,
        refresh_token: json.refresh_token,
        expires_at_ms,
        scopes,
    })
}

/// GET `/api/oauth/profile` and project the response onto our
/// [`ProfileSnapshot`]. A `None` is returned for transient failures so
/// callers can choose whether to fail the login or proceed without tier
/// info.
pub async fn fetch_profile(access_token: &str) -> Option<ProfileSnapshot> {
    let client = Client::builder().timeout(PROFILE_TIMEOUT).build().ok()?;
    let resp = client
        .get(PROFILE_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let raw: RawProfile = resp.json().await.ok()?;
    let roles = fetch_roles(&client, access_token).await;
    let plan = derive_plan(&raw);
    Some(ProfileSnapshot {
        email: raw.account.as_ref().and_then(|a| a.email.clone()),
        uuid: raw.account.as_ref().and_then(|a| a.uuid.clone()),
        plan,
        rate_limit_tier: raw
            .organization
            .as_ref()
            .and_then(|o| o.rate_limit_tier.clone()),
        full_name: raw.account.as_ref().and_then(|a| a.full_name.clone()),
        organization_uuid: raw.organization.as_ref().and_then(|o| o.uuid.clone()),
        organization_name: raw.organization.as_ref().and_then(|o| o.name.clone()),
        organization_type: raw
            .organization
            .as_ref()
            .and_then(|o| o.organization_type.clone()),
        billing_type: raw
            .organization
            .as_ref()
            .and_then(|o| o.billing_type.clone()),
        display_name: raw.account.as_ref().and_then(|a| a.display_name.clone()),
        has_claude_max: raw
            .account
            .as_ref()
            .and_then(|a| a.has_claude_max)
            .unwrap_or(false),
        has_claude_pro: raw
            .account
            .as_ref()
            .and_then(|a| a.has_claude_pro)
            .unwrap_or(false),
        has_extra_usage_enabled: raw
            .organization
            .as_ref()
            .and_then(|o| o.has_extra_usage_enabled)
            .unwrap_or(false),
        subscription_status: raw
            .organization
            .as_ref()
            .and_then(|o| o.subscription_status.clone()),
        organization_role: roles.as_ref().and_then(|r| r.organization_role.clone()),
        workspace_uuid: roles.as_ref().and_then(|r| r.workspace_uuid.clone()),
        workspace_name: roles.as_ref().and_then(|r| r.workspace_name.clone()),
        workspace_role: roles.as_ref().and_then(|r| r.workspace_role.clone()),
    })
}

/// Full login: exchange a `code#state` paste for tokens, fetch profile,
/// then persist a fully-populated [`Account`] into the manager. Returns the
/// account name on success.
pub async fn login(
    manager: &AccountManager,
    name: &str,
    raw_code_state: &str,
    verifier: &str,
    expected_state: &str,
) -> Result<String, LoginError> {
    login_with_redirect(
        manager,
        name,
        raw_code_state,
        verifier,
        expected_state,
        MANUAL_REDIRECT_URI,
    )
    .await
}

pub async fn login_with_redirect(
    manager: &AccountManager,
    name: &str,
    raw_code_state: &str,
    verifier: &str,
    expected_state: &str,
    redirect_uri: &str,
) -> Result<String, LoginError> {
    let tokens =
        exchange_code_with_redirect(raw_code_state, verifier, expected_state, redirect_uri).await?;
    login_with_tokens(manager, name, tokens).await
}

pub async fn login_with_code_and_state(
    manager: &AccountManager,
    name: &str,
    code: &str,
    returned_state: &str,
    verifier: &str,
    expected_state: &str,
    redirect_uri: &str,
) -> Result<String, LoginError> {
    let tokens = exchange_code_parts(
        code,
        Some(returned_state),
        verifier,
        expected_state,
        redirect_uri,
    )
    .await?;
    login_with_tokens(manager, name, tokens).await
}

async fn login_with_tokens(
    manager: &AccountManager,
    name: &str,
    tokens: TokenPair,
) -> Result<String, LoginError> {
    let profile = fetch_profile(&tokens.access_token)
        .await
        .unwrap_or_default();
    validate_org_restriction(&profile)?;
    let existing_accounts = manager.list_accounts().await;
    let resolved_name = resolve_account_name(&existing_accounts, name, &profile);
    let duplicate_names =
        duplicate_account_names(&existing_accounts, name, &resolved_name, &profile);

    let mut extra = serde_json::Map::new();
    if let Some(full_name) = profile.full_name {
        extra.insert("fullName".to_owned(), json!(full_name));
    }
    if let Some(display_name) = profile.display_name.clone() {
        extra.insert("displayName".to_owned(), json!(display_name));
    }
    if let Some(org_uuid) = profile.organization_uuid.clone() {
        extra.insert("organizationUuid".to_owned(), json!(org_uuid));
    }
    if let Some(org_name) = profile.organization_name.clone() {
        extra.insert("organizationName".to_owned(), json!(org_name));
    }
    if let Some(org_type) = profile.organization_type {
        extra.insert("organizationType".to_owned(), json!(org_type));
    }
    if let Some(billing_type) = profile.billing_type {
        extra.insert("billingType".to_owned(), json!(billing_type));
    }
    if let Some(subscription_status) = profile.subscription_status {
        extra.insert("subscriptionStatus".to_owned(), json!(subscription_status));
    }
    if let Some(org_role) = profile.organization_role {
        extra.insert("organizationRole".to_owned(), json!(org_role));
    }
    if let Some(workspace_uuid) = profile.workspace_uuid {
        extra.insert("workspaceUuid".to_owned(), json!(workspace_uuid));
    }
    if let Some(workspace_name) = profile.workspace_name {
        extra.insert("workspaceName".to_owned(), json!(workspace_name));
    }
    if let Some(workspace_role) = profile.workspace_role {
        extra.insert("workspaceRole".to_owned(), json!(workspace_role));
    }
    extra.insert("hasClaudeMax".to_owned(), json!(profile.has_claude_max));
    extra.insert("hasClaudePro".to_owned(), json!(profile.has_claude_pro));
    extra.insert(
        "hasExtraUsageEnabled".to_owned(),
        json!(profile.has_extra_usage_enabled),
    );

    let account = Account {
        name: resolved_name.clone(),
        refresh_token: tokens.refresh_token,
        access_token: Some(tokens.access_token),
        expires_at: Some(tokens.expires_at_ms),
        enabled: Some(true),
        rate_limit_tier: profile.rate_limit_tier,
        plan: profile.plan,
        email: profile.email,
        uuid: profile.uuid,
        added_at: Some(now_ms()),
        last_used: None,
        disabled_reason: None,
        rate_limit_reset_time: None,
        unified_status: None,
        unified_reset_at: None,
        rate_limit_type: None,
        overage_status: None,
        overage_reset_time: None,
        overage_disabled_reason: None,
        is_using_overage: None,
        utilization_5h: None,
        utilization_5h_reset_at: None,
        utilization_7d: None,
        utilization_7d_reset_at: None,
        last_usage_refresh_at: None,
        daily_usage: None,
        total_usage: None,
        extra,
    };
    manager.atomic_add_account(account).await?;
    for duplicate_name in duplicate_names {
        manager
            .atomic_remove_account(&duplicate_name)
            .await
            .map_err(|e| {
                LoginError::Transient(format!(
                    "failed to remove duplicate account {duplicate_name}: {e}"
                ))
            })?;
    }
    Ok(resolved_name)
}

// ── helpers ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: Option<f64>,
    scope: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawProfile {
    account: Option<RawAccount>,
    organization: Option<RawOrg>,
}

#[derive(Deserialize, Default)]
struct RawAccount {
    uuid: Option<String>,
    email: Option<String>,
    full_name: Option<String>,
    display_name: Option<String>,
    has_claude_max: Option<bool>,
    has_claude_pro: Option<bool>,
}

#[derive(Deserialize, Default)]
struct RawOrg {
    uuid: Option<String>,
    name: Option<String>,
    organization_type: Option<String>,
    rate_limit_tier: Option<String>,
    billing_type: Option<String>,
    has_extra_usage_enabled: Option<bool>,
    subscription_status: Option<String>,
}

#[derive(Deserialize)]
struct RawRoles {
    organization_role: Option<String>,
    workspace_uuid: Option<String>,
    workspace_name: Option<String>,
    workspace_role: Option<String>,
}

fn derive_plan(raw: &RawProfile) -> Option<String> {
    let org_type = raw
        .organization
        .as_ref()
        .and_then(|o| o.organization_type.as_deref());
    let has_max = raw
        .account
        .as_ref()
        .and_then(|a| a.has_claude_max)
        .unwrap_or(false);
    let has_pro = raw
        .account
        .as_ref()
        .and_then(|a| a.has_claude_pro)
        .unwrap_or(false);
    if org_type == Some("claude_max") || has_max {
        return Some("claude_max".to_owned());
    }
    if org_type == Some("claude_pro") || has_pro {
        return Some("claude_pro".to_owned());
    }
    None
}

async fn fetch_roles(client: &Client, access_token: &str) -> Option<RawRoles> {
    let resp = client
        .get(ROLES_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

fn validate_org_restriction(profile: &ProfileSnapshot) -> Result<(), LoginError> {
    let Some(raw) = forced_org_uuid() else {
        return Ok(());
    };
    let allowed: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if allowed.is_empty() {
        return Err(LoginError::Permanent(
            "ANTHROPIC_FORCE_LOGIN_ORG_UUID is set but empty".to_owned(),
        ));
    }
    let Some(org_uuid) = profile.organization_uuid.as_deref() else {
        return Err(LoginError::Permanent(format!(
            "account has no organization UUID, allowed: {}",
            allowed.join(", ")
        )));
    };
    if allowed.contains(&org_uuid) {
        return Ok(());
    }
    Err(LoginError::Permanent(format!(
        "account org {org_uuid} is not in allowed list: {}",
        allowed.join(", ")
    )))
}

fn client_id() -> String {
    std::env::var("CLAUDE_CODE_OAUTH_CLIENT_ID")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_CLIENT_ID.to_owned())
}

fn force_login_method() -> Option<String> {
    std::env::var("ANTHROPIC_FORCE_LOGIN_METHOD")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| matches!(s.as_str(), "claudeai" | "console"))
}

fn forced_org_uuid() -> Option<String> {
    std::env::var("ANTHROPIC_FORCE_LOGIN_ORG_UUID")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn resolve_account_name(
    existing: &[Account],
    requested_name: &str,
    profile: &ProfileSnapshot,
) -> String {
    if let Some(email) = profile.email.as_deref() {
        if let Some(account) = existing.iter().find(|account| account.name == email) {
            return account.name.clone();
        }
        if let Some(account) = existing
            .iter()
            .find(|account| account.email.as_deref() == Some(email))
        {
            return account.name.clone();
        }
    }
    if let Some(account) = existing.iter().find(|account| {
        account.name == requested_name && account_matches_profile_identity(account, profile)
    }) {
        return account.name.clone();
    }
    profile
        .email
        .clone()
        .filter(|email| !email.trim().is_empty())
        .unwrap_or_else(|| requested_name.to_owned())
}

fn duplicate_account_names(
    existing: &[Account],
    requested_name: &str,
    resolved_name: &str,
    profile: &ProfileSnapshot,
) -> Vec<String> {
    existing
        .iter()
        .filter(|account| account.name != resolved_name)
        .filter(|account| {
            account.name == requested_name || account_matches_profile_identity(account, profile)
        })
        .map(|account| account.name.clone())
        .collect()
}

fn account_matches_profile_identity(account: &Account, profile: &ProfileSnapshot) -> bool {
    if let Some(email) = profile.email.as_deref() {
        if account.name == email || account.email.as_deref() == Some(email) {
            return true;
        }
    }
    let Some(org_uuid) = profile.organization_uuid.as_deref() else {
        return false;
    };
    account
        .extra
        .get("organizationUuid")
        .and_then(|value| value.as_str())
        == Some(org_uuid)
}

fn parse_oauth_error(body: &str) -> (Option<String>, String) {
    #[derive(Deserialize)]
    struct ErrJson {
        error: Option<String>,
        error_description: Option<String>,
    }
    if let Ok(e) = serde_json::from_str::<ErrJson>(body) {
        let detail = format!(
            "{}: {}",
            e.error.as_deref().unwrap_or("unknown_error"),
            e.error_description.as_deref().unwrap_or("no description")
        );
        return (e.error, detail);
    }
    (None, body.chars().take(200).collect())
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 4);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn generate_random_b64url(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    for byte in buf.iter_mut() {
        *byte = rand::random::<u8>();
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

fn sha256_b64url(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// Constant-time byte-slice equality. Length mismatch still drives the loop
/// to completion to avoid leaking length information through timing.
fn timing_safe_equal(a: &[u8], b: &[u8]) -> bool {
    let max = a.len().max(b.len());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..max {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        diff |= av ^ bv;
    }
    diff == 0
}

/// `sk-ant-oat01-…` (OAuth access token) shape per opencode
/// `oauth/validation.ts`. Length is permissive — Anthropic occasionally
/// rotates the suffix length.
fn is_valid_access_token(s: &str) -> bool {
    s.starts_with("sk-ant-oat01-") && s.len() >= 32
}

/// `sk-ant-ort01-…` (OAuth refresh token) per current Claude Code / opencode
/// validation. Access and refresh tokens do NOT share a prefix.
fn is_valid_refresh_token(s: &str) -> bool {
    s.starts_with("sk-ant-ort01-") && s.len() >= 32
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: authorize() produces a URL with all required params and a
    // matching verifier/challenge pair.
    #[test]
    fn authorize_produces_valid_url_normal() {
        let req = authorize();
        assert!(req.url.starts_with(CLAUDE_AI_AUTHORIZE_URL));
        assert!(req.url.contains("client_id="));
        assert!(req.url.contains("code_challenge="));
        assert!(req.url.contains("code_challenge_method=S256"));
        assert!(
            req.url
                .contains(&format!("state={}", url_encode(&req.state)))
        );
        assert!(req.url.contains(&url_encode(MANUAL_REDIRECT_URI)));
        assert_eq!(req.verifier.len(), 43);
        assert_eq!(req.state.len(), 43);
        assert_ne!(req.verifier, req.state);
    }

    #[test]
    fn authorize_localhost_redirect_normal() {
        let req = authorize_with_redirect(RedirectTarget::Localhost(43123));
        assert!(
            req.url
                .contains(&url_encode("http://localhost:43123/callback"))
        );
        assert_eq!(req.redirect_uri, "http://localhost:43123/callback");
    }

    // Robust: SHA-256 challenge for a known verifier matches RFC 7636 §A.4
    // example. Guards against a sloppy base64 / hash refactor.
    #[test]
    fn pkce_known_vector_robust() {
        // RFC 7636 §A.4 example
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(sha256_b64url(verifier), expected);
    }

    // Edge: a paste with NO `#state` suffix is treated as missing-state and
    // returns StateMismatch. Defends against the user pasting only the code
    // portion of the callback URL.
    #[test]
    fn missing_state_in_paste_edge() {
        let (code, state) = split_callback_paste("abcXYZ123");
        assert_eq!(code, "abcXYZ123");
        assert!(state.is_none());
    }

    // Normal: `code#state` paste is split correctly.
    #[test]
    fn paste_split_normal() {
        let (code, state) = split_callback_paste("the_code#the_state");
        assert_eq!(code, "the_code");
        assert_eq!(state.as_deref(), Some("the_state"));
    }

    // Robust: timing-safe compare returns false for mismatched lengths.
    #[test]
    fn timing_safe_robust() {
        assert!(timing_safe_equal(b"abc", b"abc"));
        assert!(!timing_safe_equal(b"abc", b"abd"));
        assert!(!timing_safe_equal(b"abc", b"abcd"));
        assert!(!timing_safe_equal(b"", b"x"));
        assert!(timing_safe_equal(b"", b""));
    }

    // Robust: token shape validator rejects API keys (sk-ant-api01) and
    // accepts OAuth tokens (sk-ant-oat01).
    #[test]
    fn token_shape_robust() {
        assert!(is_valid_access_token(
            "sk-ant-oat01-aaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(is_valid_refresh_token(
            "sk-ant-ort01-aaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(!is_valid_access_token(
            "sk-ant-api01-aaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(!is_valid_refresh_token(
            "sk-ant-oat01-aaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(!is_valid_access_token("too-short"));
        assert!(!is_valid_access_token(""));
    }

    #[test]
    fn resolve_account_name_prefers_existing_identity_normal() {
        let existing = vec![
            test_account("cole@unwrap.rs", Some("cole@unwrap.rs"), Some("org-unwrap")),
            test_account(
                "cole.leavitt@fiwealth.com",
                Some("cole.leavitt@fiwealth.com"),
                Some("org-fiwealth"),
            ),
        ];
        let profile = ProfileSnapshot {
            email: Some("cole.leavitt@fiwealth.com".to_owned()),
            organization_uuid: Some("org-fiwealth".to_owned()),
            ..ProfileSnapshot::default()
        };
        assert_eq!(
            resolve_account_name(&existing, "personal", &profile),
            "cole.leavitt@fiwealth.com"
        );
    }

    #[test]
    fn duplicate_account_names_collects_aliases_for_same_identity_normal() {
        let existing = vec![
            test_account(
                "personal",
                Some("cole.leavitt@fiwealth.com"),
                Some("org-fiwealth"),
            ),
            test_account(
                "cole.leavitt@fiwealth.com",
                Some("cole.leavitt@fiwealth.com"),
                Some("org-fiwealth"),
            ),
            test_account("cole@unwrap.rs", Some("cole@unwrap.rs"), Some("org-unwrap")),
        ];
        let profile = ProfileSnapshot {
            email: Some("cole.leavitt@fiwealth.com".to_owned()),
            organization_uuid: Some("org-fiwealth".to_owned()),
            ..ProfileSnapshot::default()
        };
        assert_eq!(
            duplicate_account_names(&existing, "personal", "cole.leavitt@fiwealth.com", &profile,),
            vec!["personal".to_owned()]
        );
    }

    // Edge: empty code in a `#state` paste rejects with EmptyCode early
    // (don't even attempt the round-trip).
    #[tokio::test]
    async fn empty_code_rejected_edge() {
        let res = exchange_code("#somestate", "v", "somestate").await;
        assert!(matches!(res, Err(LoginError::EmptyCode)));
    }

    // Edge: state mismatch is rejected before any network is touched.
    #[tokio::test]
    async fn state_mismatch_rejected_edge() {
        let res = exchange_code("code#wrongstate", "v", "rightstate").await;
        assert!(matches!(res, Err(LoginError::StateMismatch)));
    }

    // Robust: derive_plan picks claude_max when org type matches even if
    // both has_claude_max/has_claude_pro flags are false (org type wins).
    #[test]
    fn derive_plan_org_type_wins_robust() {
        let raw = RawProfile {
            account: Some(RawAccount {
                has_claude_max: Some(false),
                has_claude_pro: Some(true),
                ..Default::default()
            }),
            organization: Some(RawOrg {
                organization_type: Some("claude_max".to_owned()),
                ..Default::default()
            }),
        };
        assert_eq!(derive_plan(&raw).as_deref(), Some("claude_max"));
    }

    // Normal: smoke test that a fully-populated Account roundtrips through
    // atomic_add_account → list_accounts.
    #[tokio::test]
    async fn account_persists_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        let acct = Account {
            name: "test".to_owned(),
            refresh_token: "sk-ant-ort01-stub-refresh-token-string".to_owned(),
            access_token: Some("sk-ant-oat01-stub-access-token-string".to_owned()),
            expires_at: Some(now_ms() + 3_600_000),
            enabled: Some(true),
            rate_limit_tier: Some("claude_max_5x".to_owned()),
            plan: Some("claude_max".to_owned()),
            email: Some("u@example.com".to_owned()),
            uuid: None,
            added_at: Some(now_ms()),
            last_used: None,
            disabled_reason: None,
            rate_limit_reset_time: None,
            unified_status: None,
            unified_reset_at: None,
            rate_limit_type: None,
            overage_status: None,
            overage_reset_time: None,
            overage_disabled_reason: None,
            is_using_overage: None,
            utilization_5h: None,
            utilization_5h_reset_at: None,
            utilization_7d: None,
            utilization_7d_reset_at: None,
            last_usage_refresh_at: None,
            daily_usage: None,
            total_usage: None,
            extra: serde_json::Map::new(),
        };
        mgr.atomic_add_account(acct).await.unwrap();
        let listed = mgr.list_accounts().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "test");
    }

    fn test_account(name: &str, email: Option<&str>, organization_uuid: Option<&str>) -> Account {
        let mut extra = serde_json::Map::new();
        if let Some(org_uuid) = organization_uuid {
            extra.insert("organizationUuid".to_owned(), json!(org_uuid));
        }
        Account {
            name: name.to_owned(),
            refresh_token: "sk-ant-ort01-stub-refresh-token-string".to_owned(),
            access_token: Some("sk-ant-oat01-stub-access-token-string".to_owned()),
            expires_at: Some(now_ms() + 3_600_000),
            enabled: Some(true),
            rate_limit_tier: Some("claude_max_5x".to_owned()),
            plan: Some("claude_max".to_owned()),
            email: email.map(str::to_owned),
            uuid: None,
            added_at: Some(now_ms()),
            last_used: None,
            disabled_reason: None,
            rate_limit_reset_time: None,
            unified_status: None,
            unified_reset_at: None,
            rate_limit_type: None,
            overage_status: None,
            overage_reset_time: None,
            overage_disabled_reason: None,
            is_using_overage: None,
            utilization_5h: None,
            utilization_5h_reset_at: None,
            utilization_7d: None,
            utilization_7d_reset_at: None,
            last_usage_refresh_at: None,
            daily_usage: None,
            total_usage: None,
            extra,
        }
    }
}
