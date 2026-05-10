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
use sha2::{Digest, Sha256};

use super::anthropic_accounts::{Account, AccountManager, now_ms};

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const AUTHORIZE_URL: &str = "https://platform.claude.com/oauth/authorize";
const REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";

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
}

/// Successful token-exchange result. `expires_at_ms` is already adjusted for
/// the 30s skew opencode applies (`Date.now() + expires_in*1000 - 30000`).
#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: u64,
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
    pub organization_uuid: Option<String>,
    pub organization_name: Option<String>,
    pub display_name: Option<String>,
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
    let verifier = generate_random_b64url(32);
    let challenge = sha256_b64url(&verifier);
    let state = generate_random_b64url(32);
    let url = format!(
        "{AUTHORIZE_URL}?code=true&client_id={cid}&response_type=code&redirect_uri={redir}\
         &scope={scope}&code_challenge={chal}&code_challenge_method=S256&state={st}",
        cid = url_encode(CLIENT_ID),
        redir = url_encode(REDIRECT_URI),
        scope = url_encode(AUTHORIZE_SCOPES),
        chal = url_encode(&challenge),
        st = url_encode(&state),
    );
    AuthorizeRequest { url, verifier, state }
}

/// Parse a `code#state` paste from the Anthropic callback page into its
/// constituent parts. Returns `(code, state)` — `state` is `None` if the
/// paste lacked the `#…` suffix.
fn split_callback_paste(raw: &str) -> (String, Option<String>) {
    if let Some(hash_idx) = raw.rfind('#') {
        let (code, rest) = raw.split_at(hash_idx);
        let state = rest.trim_start_matches('#').trim();
        let state = if state.is_empty() { None } else { Some(state.to_owned()) };
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
pub async fn exchange_code(
    raw_code_state: &str,
    verifier: &str,
    expected_state: &str,
) -> Result<TokenPair, LoginError> {
    let (code, returned_state) = split_callback_paste(raw_code_state);
    if code.is_empty() {
        return Err(LoginError::EmptyCode);
    }
    let returned_state = returned_state.ok_or(LoginError::StateMismatch)?;
    if !timing_safe_equal(returned_state.as_bytes(), expected_state.as_bytes()) {
        return Err(LoginError::StateMismatch);
    }

    let client = Client::builder().timeout(OAUTH_TIMEOUT).build().map_err(|e| {
        LoginError::Transient(format!("reqwest builder: {e}"))
    })?;

    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": REDIRECT_URI,
        "client_id": CLIENT_ID,
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
        return Err(LoginError::Transient(format!("invalid expires_in={expires_in}")));
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
    let plan = derive_plan(&raw);
    Some(ProfileSnapshot {
        email: raw.account.as_ref().and_then(|a| a.email.clone()),
        uuid: raw.account.as_ref().and_then(|a| a.uuid.clone()),
        plan,
        rate_limit_tier: raw
            .organization
            .as_ref()
            .and_then(|o| o.rate_limit_tier.clone()),
        organization_uuid: raw.organization.as_ref().and_then(|o| o.uuid.clone()),
        organization_name: raw.organization.as_ref().and_then(|o| o.name.clone()),
        display_name: raw.account.as_ref().and_then(|a| a.display_name.clone()),
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
    let tokens = exchange_code(raw_code_state, verifier, expected_state).await?;
    let profile = fetch_profile(&tokens.access_token).await.unwrap_or_default();

    let account = Account {
        name: name.to_owned(),
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
        extra: serde_json::Map::new(),
    };
    manager.atomic_add_account(account).await?;
    Ok(name.to_owned())
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

/// `sk-ant-oat01-…` (OAuth refresh token) per opencode validation. Note
/// Anthropic uses the same `oat01` prefix for both access and refresh —
/// the difference is which endpoint accepts which.
fn is_valid_refresh_token(s: &str) -> bool {
    s.starts_with("sk-ant-oat01-") && s.len() >= 32
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
        assert!(req.url.starts_with(AUTHORIZE_URL));
        assert!(req.url.contains("client_id="));
        assert!(req.url.contains("code_challenge="));
        assert!(req.url.contains("code_challenge_method=S256"));
        assert!(req.url.contains(&format!("state={}", url_encode(&req.state))));
        assert_eq!(req.verifier.len(), 43);
        assert_eq!(req.state.len(), 43);
        assert_ne!(req.verifier, req.state);
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
        assert!(!is_valid_access_token(
            "sk-ant-api01-aaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));
        assert!(!is_valid_access_token("too-short"));
        assert!(!is_valid_access_token(""));
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
            refresh_token: "sk-ant-oat01-stub-refresh-token-string".to_owned(),
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
            extra: serde_json::Map::new(),
        };
        mgr.atomic_add_account(acct).await.unwrap();
        let listed = mgr.list_accounts().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "test");
    }
}
