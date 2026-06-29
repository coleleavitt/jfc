//! Antigravity (Google) OAuth provider.
//!
//! Ported from the `opencode-google-antigravity-auth` plugin
//! (`research/opencode-google-antigravity-auth`). Authenticates with Google
//! via OAuth 2.0 + PKCE and talks to Google's internal **Code Assist** API
//! (`cloudcode-pa.googleapis.com/v1internal`), which fronts both Gemini 3.x
//! and Claude-via-Gemini models for Antigravity / AI Pro subscribers.
//!
//! Layering mirrors `codex_oauth.rs`:
//!   * constants (mirrored byte-for-byte from `src/constants.ts`),
//!   * the OAuth flow (`authorize_url` / `exchange` / `refresh` /
//!     `fetch_account_info`, ported from `src/antigravity/oauth.ts`),
//!   * the [`AntigravityOAuthProvider`] struct + token lifecycle,
//!   * the [`Provider`] impl (model catalogue + Code Assist streaming).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use base64::Engine;
use jfc_auth::oauth_core::{AuthMethod, TokenStore, generate_pkce, now_secs};
use jfc_provider::{
    CompletionResponse, EventStream, ModelInfo, Provider, ProviderMessage, StreamConvention,
    StreamOptions,
};

// ─── Constants (mirror of src/constants.ts) ──────────────────────────────────

const PROVIDER_ID: &str = "antigravity";
const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const CALLBACK_PORT: u16 = 36742;

/// OAuth scopes requested from Google. Mirrors `ANTIGRAVITY_SCOPES`.
const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/cclog",
    "https://www.googleapis.com/auth/experimentsandconfigs",
];

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v1/userinfo?alt=json";

/// Code Assist API root endpoints, in fallback order (daily → autopush → prod).
/// Mirrors `CODE_ASSIST_ENDPOINT_FALLBACKS`.
const CODE_ASSIST_ENDPOINTS: &[&str] = &[
    "https://daily-cloudcode-pa.sandbox.googleapis.com",
    "https://autopush-cloudcode-pa.sandbox.googleapis.com",
    "https://cloudcode-pa.googleapis.com",
];
const CODE_ASSIST_API_VERSION: &str = "v1internal";

const API_CLIENT: &str = "google-cloud-sdk vscode_cloudshelleditor/0.1";
const CLIENT_METADATA: &str =
    r#"{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}"#;

const DEFAULT_EXPIRES_IN: u64 = 3600;

/// Build the Antigravity `User-Agent` (`antigravity/<ver> <platform>/<arch>`),
/// mirroring `getAntigravityPlatform()`. Falls back to `linux/amd64` for
/// unrecognized targets.
fn user_agent() -> String {
    let plat = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "windows",
        "linux" => "linux",
        _ => "linux",
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        "x86" => "386",
        _ => "amd64",
    };
    format!("antigravity/1.15.8 {plat}/{arch}")
}

/// The `localhost` redirect URI the Google consent screen calls back to.
fn redirect_uri() -> String {
    format!("http://localhost:{CALLBACK_PORT}/oauth-callback")
}

// ─── OAuth flow (mirror of src/antigravity/oauth.ts) ─────────────────────────

/// Account tier discovered from `loadCodeAssist`. Subscription-covered usage
/// is `Paid`; everything else (including the legacy/free tiers) is `Free`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountTier {
    Free,
    Paid,
}

/// PKCE + project state carried through the OAuth round-trip, packed into the
/// `state` query param as base64url(JSON). Mirrors `encodeState`/`decodeState`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthState {
    verifier: String,
    #[serde(default, rename = "projectId")]
    project_id: String,
}

/// A started authorization: the consent URL to open plus the PKCE verifier the
/// caller must hand back to [`exchange`].
#[derive(Debug, Clone)]
pub struct AntigravityAuthorization {
    pub url: String,
    pub verifier: String,
    pub project_id: String,
}

/// Tokens + metadata persisted for an authenticated account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityTokenSet {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub project_id: String,
    pub email: Option<String>,
    pub tier: AccountTier,
}

fn encode_state(state: &AuthState) -> String {
    let json = serde_json::to_vec(state).unwrap_or_default();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
}

fn decode_state(state: &str) -> anyhow::Result<AuthState> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(state.as_bytes())
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(state.as_bytes()))?;
    let parsed: AuthState = serde_json::from_slice(&bytes)?;
    if parsed.verifier.is_empty() {
        anyhow::bail!("missing PKCE verifier in OAuth state");
    }
    Ok(parsed)
}

/// Build the Google OAuth consent URL with PKCE. Mirrors `authorizeAntigravity`.
pub fn authorize_url(project_id: &str) -> AntigravityAuthorization {
    let pkce = generate_pkce();
    let state = encode_state(&AuthState {
        verifier: pkce.code_verifier.clone(),
        project_id: project_id.to_owned(),
    });
    let scope = SCOPES.join(" ");
    let mut url = url::Url::parse(GOOGLE_AUTH_URL).expect("static auth url");
    url.query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri())
        .append_pair("scope", &scope)
        .append_pair("code_challenge", &pkce.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    AntigravityAuthorization {
        url: url.to_string(),
        verifier: pkce.code_verifier,
        project_id: project_id.to_owned(),
    }
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// Exchange an authorization `code` (+ the `state` that carries the PKCE
/// verifier + project id) for tokens. Mirrors `exchangeAntigravity`.
pub async fn exchange(
    client: &reqwest::Client,
    code: &str,
    state: &str,
) -> anyhow::Result<AntigravityTokenSet> {
    let AuthState {
        verifier,
        project_id,
    } = decode_state(state)?;

    let redirect = redirect_uri();
    let body = serde_urlencoded::to_string([
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("code", code),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect.as_str()),
        ("code_verifier", verifier.as_str()),
    ])?;
    let token: GoogleTokenResponse = client
        .post(GOOGLE_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let refresh_token = token
        .refresh_token
        .ok_or_else(|| anyhow::anyhow!("missing refresh token in Google token response"))?;

    let email = fetch_email(client, &token.access_token).await;

    let account = fetch_account_info(client, &token.access_token).await;
    let effective_project = if project_id.is_empty() {
        account.0
    } else {
        project_id
    };

    Ok(AntigravityTokenSet {
        access_token: token.access_token,
        refresh_token,
        expires_at: now_secs() + token.expires_in.unwrap_or(DEFAULT_EXPIRES_IN),
        project_id: effective_project,
        email,
        tier: account.1,
    })
}

/// Refresh an access token via the standard Google `refresh_token` grant.
pub async fn refresh(
    client: &reqwest::Client,
    refresh_token: &str,
) -> anyhow::Result<(String, u64)> {
    let body = serde_urlencoded::to_string([
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ])?;
    let token: GoogleTokenResponse = client
        .post(GOOGLE_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok((
        token.access_token,
        now_secs() + token.expires_in.unwrap_or(DEFAULT_EXPIRES_IN),
    ))
}

async fn fetch_email(client: &reqwest::Client, access_token: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct UserInfo {
        email: Option<String>,
    }
    let resp = client
        .get(GOOGLE_USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<UserInfo>().await.ok()?.email
}

/// Discover the user's default project id + tier by calling
/// `{endpoint}/v1internal:loadCodeAssist` across the endpoint fallback chain.
/// Mirrors `fetchAccountInfo`. Returns `("", Free)` if nothing resolves.
pub async fn fetch_account_info(
    client: &reqwest::Client,
    access_token: &str,
) -> (String, AccountTier) {
    let body = serde_json::json!({
        "metadata": {
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI",
        }
    });

    let mut tier = AccountTier::Free;
    for endpoint in CODE_ASSIST_ENDPOINTS {
        let url = format!("{endpoint}/{CODE_ASSIST_API_VERSION}:loadCodeAssist");
        let Ok(resp) = client
            .post(&url)
            .bearer_auth(access_token)
            .header("X-Goog-Api-Client", API_CLIENT)
            .header("Client-Metadata", CLIENT_METADATA)
            .json(&body)
            .send()
            .await
        else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(data) = resp.json::<serde_json::Value>().await else {
            continue;
        };

        let (project_id, detected) = parse_account_info(&data);
        if detected == AccountTier::Paid {
            tier = AccountTier::Paid;
        }
        if !project_id.is_empty() {
            return (project_id, tier);
        }
    }
    (String::new(), tier)
}

/// Pure parser for a `loadCodeAssist` JSON body → (project_id, tier). Split out
/// so it is unit-testable without a live endpoint.
fn parse_account_info(data: &serde_json::Value) -> (String, AccountTier) {
    let mut project_id = String::new();
    match &data["cloudaicompanionProject"] {
        serde_json::Value::String(s) if !s.is_empty() => project_id = s.clone(),
        serde_json::Value::Object(obj) => {
            if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
                project_id = id.to_owned();
            }
        }
        _ => {}
    }

    let mut tier = AccountTier::Free;
    if let Some(tiers) = data["allowedTiers"].as_array()
        && let Some(default) = tiers
            .iter()
            .find(|t| t["isDefault"].as_bool() == Some(true))
        && let Some(id) = default["id"].as_str()
        && id != "legacy-tier"
        && !id.contains("free")
        && !id.contains("zero")
    {
        tier = AccountTier::Paid;
    }
    if let Some(paid) = data["paidTier"]["id"].as_str()
        && !paid.contains("free")
        && !paid.contains("zero")
    {
        tier = AccountTier::Paid;
    }
    (project_id, tier)
}

// ─── Provider ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AntigravityOAuthProvider {
    client: reqwest::Client,
    store: TokenStore,
}

impl Default for AntigravityOAuthProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AntigravityOAuthProvider {
    pub fn new() -> Self {
        Self {
            client: jfc_provider::http::streaming_client(),
            store: TokenStore::default_path().into(),
        }
    }

    #[allow(dead_code)]
    pub fn with_store(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            client: jfc_provider::http::streaming_client(),
            store: TokenStore::new(path),
        }
    }

    pub fn has_usable_config(&self) -> bool {
        matches!(
            self.store.get(PROVIDER_ID),
            Ok(Some(AuthMethod::OAuth { .. }))
        )
    }

    pub fn store_path(&self) -> &std::path::Path {
        self.store.path()
    }

    /// Build the authorization URL + PKCE verifier for `/login antigravity`.
    pub fn authorize(project_id: &str) -> AntigravityAuthorization {
        authorize_url(project_id)
    }

    /// Complete the OAuth round-trip and persist the resulting tokens.
    pub async fn complete_login(&self, code: &str, state: &str) -> anyhow::Result<()> {
        let tokens = exchange(&self.client, code, state).await?;
        self.persist(&tokens)?;
        Ok(())
    }

    fn persist(&self, tokens: &AntigravityTokenSet) -> anyhow::Result<()> {
        // `account_id` packs `email|projectId|tier` so a single AuthMethod row
        // round-trips all the metadata the Code Assist requests need, matching
        // the TS plugin's `{refresh}|{projectId}` convention but richer.
        let account_id = format!(
            "{}|{}|{}",
            tokens.email.as_deref().unwrap_or(""),
            tokens.project_id,
            match tokens.tier {
                AccountTier::Paid => "paid",
                AccountTier::Free => "free",
            }
        );
        self.store.set(
            PROVIDER_ID,
            AuthMethod::OAuth {
                access_token: tokens.access_token.clone(),
                refresh_token: tokens.refresh_token.clone(),
                expires_at: tokens.expires_at,
                account_id: Some(account_id),
            },
        )?;
        Ok(())
    }

    /// The static Antigravity model catalogue. Cost is 0 — usage is covered by
    /// the Google AI Pro / Antigravity subscription, like the TS plugin.
    fn antigravity_models() -> Vec<ModelInfo> {
        let mk = |id: &str, name: &str, ctx: usize, out: usize| {
            let mut m = ModelInfo::new(id, name, PROVIDER_ID);
            m.context_window_tokens = Some(ctx);
            m.max_output_tokens = Some(out);
            m.input_cost = Some(0.0);
            m.output_cost = Some(0.0);
            m
        };
        vec![
            // Gemini 3.x (native via Antigravity)
            mk("gemini-3.5-flash", "Gemini 3.5 Flash", 1_048_576, 65_536),
            mk(
                "gemini-3.1-pro-preview",
                "Gemini 3.1 Pro Preview",
                1_048_576,
                65_536,
            ),
            mk(
                "gemini-3-pro-preview",
                "Gemini 3 Pro Preview",
                1_048_576,
                65_536,
            ),
            mk(
                "gemini-3-flash-preview",
                "Gemini 3 Flash Preview",
                1_048_576,
                65_536,
            ),
            // Gemini 2.5 (native via Antigravity)
            mk("gemini-2.5-flash", "Gemini 2.5 Flash", 1_048_576, 65_536),
            mk("gemini-2.5-pro", "Gemini 2.5 Pro", 1_048_576, 65_536),
            // Claude via Antigravity
            mk(
                "gemini-claude-sonnet-4-5",
                "Claude Sonnet 4.5 (via Antigravity)",
                200_000,
                64_000,
            ),
            mk(
                "gemini-claude-sonnet-4-5-thinking",
                "Claude Sonnet 4.5 Thinking (via Antigravity)",
                200_000,
                64_000,
            ),
            mk(
                "gemini-claude-opus-4-5-thinking",
                "Claude Opus 4.5 Thinking (via Antigravity)",
                200_000,
                64_000,
            ),
        ]
    }

    /// The Code Assist endpoint to target (primary = daily sandbox).
    fn endpoint(&self) -> &'static str {
        CODE_ASSIST_ENDPOINTS[0]
    }

    /// Pull the live access token + persisted project id out of the store.
    /// `account_id` was packed as `{email}|{project}|{tier}` at login time
    /// (see [`Self::persist`]) — we split it back out here.
    fn access_token_and_project(&self) -> anyhow::Result<(String, String)> {
        let Some(AuthMethod::OAuth {
            access_token,
            account_id,
            ..
        }) = self.store.get(PROVIDER_ID)?
        else {
            anyhow::bail!("not logged in to Antigravity \u{2014} run `/login antigravity` first");
        };
        let project_id = account_id
            .as_deref()
            .and_then(|s| s.split('|').nth(1))
            .map(str::to_owned)
            .unwrap_or_default();
        if project_id.is_empty() {
            anyhow::bail!(
                "Antigravity login is missing a project id \u{2014} re-run \
                 `/login antigravity` so loadCodeAssist can resolve one"
            );
        }
        Ok((access_token, project_id))
    }
}

impl jfc_provider::seal::Sealed for AntigravityOAuthProvider {}

#[async_trait]
impl Provider for AntigravityOAuthProvider {
    fn name(&self) -> &str {
        PROVIDER_ID
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Self::antigravity_models()
    }

    fn stream_convention(&self) -> StreamConvention {
        // Code Assist returns Gemini-style `generateContent` SSE; the renderer
        // treats it like the Gemini convention.
        StreamConvention::AnthropicNative
    }

    fn http_client(&self) -> Option<reqwest::Client> {
        Some(self.client.clone())
    }

    fn warmup_url(&self) -> Option<String> {
        // Warm the primary Code Assist endpoint (daily sandbox → falls back to prod).
        Some(CODE_ASSIST_ENDPOINTS[0].to_owned())
    }

    async fn ensure_auth(&self) -> anyhow::Result<()> {
        let Some(AuthMethod::OAuth {
            refresh_token,
            expires_at,
            account_id,
            ..
        }) = self.store.get(PROVIDER_ID)?
        else {
            anyhow::bail!("not logged in to Antigravity — run `/login antigravity` first",);
        };

        let method = AuthMethod::OAuth {
            access_token: String::new(),
            refresh_token: refresh_token.clone(),
            expires_at,
            account_id: account_id.clone(),
        };
        if method.is_expired_or_expiring(now_secs()) {
            match refresh(&self.client, &refresh_token).await {
                Ok((access_token, new_expiry)) => {
                    self.store.set(
                        PROVIDER_ID,
                        AuthMethod::OAuth {
                            access_token,
                            refresh_token,
                            expires_at: new_expiry,
                            account_id,
                        },
                    )?;
                }
                Err(e) => {
                    let msg = e.to_string();
                    // If the refresh token was revoked, clear stored credentials
                    // so the user doesn't get stuck in a broken refresh loop.
                    if msg.contains("invalid_grant") || msg.contains("Token has been revoked") {
                        tracing::warn!(
                            target: "jfc::provider::antigravity",
                            "refresh token revoked by Google — clearing stored credentials"
                        );
                        self.store.remove(PROVIDER_ID).ok();
                        anyhow::bail!(
                            "Google revoked the stored refresh token. \
                             Run `/login antigravity` to reauthenticate."
                        );
                    }
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(Some(AuthMethod::OAuth { access_token, .. })) = self.store.get(PROVIDER_ID)
            && let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {access_token}"))
        {
            headers.insert(reqwest::header::AUTHORIZATION, v);
        }
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&user_agent()) {
            headers.insert(reqwest::header::USER_AGENT, v);
        }
        headers.insert(
            "X-Goog-Api-Client",
            reqwest::header::HeaderValue::from_static(API_CLIENT),
        );
        headers.insert(
            "Client-Metadata",
            reqwest::header::HeaderValue::from_static(CLIENT_METADATA),
        );
        headers
    }

    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        // Refresh tokens if they're stale, then pull the access token + the
        // project id we encoded into `account_id` at login time.
        self.ensure_auth().await?;
        let (access_token, project_id) = self.access_token_and_project()?;

        // Build the Code Assist envelope (project / model / requestId /
        // request{contents, tools, generationConfig, …}). `build_request`
        // auto-dispatches between the Gemini-native and Claude-via-
        // Antigravity body shapes by inspecting the model id.
        let body = super::antigravity_transform::build_request(&project_id, &messages, options)?;
        let url = format!(
            "{}/{CODE_ASSIST_API_VERSION}:streamGenerateContent?alt=sse",
            self.endpoint(),
        );

        tracing::debug!(
            target: "jfc::provider::antigravity",
            url = %url,
            model = %options.model.as_str(),
            messages = messages.len(),
            "POST streamGenerateContent"
        );

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&access_token)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .header(reqwest::header::USER_AGENT, user_agent())
            .header("X-Goog-Api-Client", API_CLIENT)
            .header("Client-Metadata", CLIENT_METADATA)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Antigravity Code Assist error {status}: {text}");
        }

        Ok(super::antigravity_transform::into_event_stream(resp))
    }

    async fn complete(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<CompletionResponse> {
        self.ensure_auth().await?;
        anyhow::bail!(
            "Antigravity complete() is not wired yet — non-streaming completion \
             would route to v1internal:generateContent; use stream() for now"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_has_antigravity_prefix_normal() {
        let ua = user_agent();
        assert!(ua.starts_with("antigravity/1.15.8 "), "got {ua}");
        assert!(ua.contains('/'));
    }

    #[test]
    fn authorize_url_has_pkce_and_offline_params_normal() {
        let auth = authorize_url("");
        let parsed = url::Url::parse(&auth.url).unwrap();
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q.get("client_id").map(String::as_str), Some(CLIENT_ID));
        assert_eq!(q.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            q.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(q.get("access_type").map(String::as_str), Some("offline"));
        assert_eq!(q.get("prompt").map(String::as_str), Some("consent"));
        assert!(q.contains_key("code_challenge"));
        assert!(q.contains_key("state"));
        assert!(!auth.verifier.is_empty());
    }

    #[test]
    fn state_encode_decode_round_trips_normal() {
        let original = AuthState {
            verifier: "test-verifier-123".into(),
            project_id: "my-project".into(),
        };
        let encoded = encode_state(&original);
        let decoded = decode_state(&encoded).unwrap();
        assert_eq!(decoded.verifier, original.verifier);
        assert_eq!(decoded.project_id, original.project_id);
    }

    #[test]
    fn decode_state_rejects_missing_verifier_robust() {
        let bad = encode_state(&AuthState {
            verifier: String::new(),
            project_id: "p".into(),
        });
        assert!(decode_state(&bad).is_err());
    }

    #[test]
    fn parse_account_info_extracts_project_string_normal() {
        let data = serde_json::json!({ "cloudaicompanionProject": "proj-abc" });
        let (project, tier) = parse_account_info(&data);
        assert_eq!(project, "proj-abc");
        assert_eq!(tier, AccountTier::Free);
    }

    #[test]
    fn parse_account_info_extracts_project_object_normal() {
        let data = serde_json::json!({ "cloudaicompanionProject": { "id": "proj-xyz" } });
        let (project, _) = parse_account_info(&data);
        assert_eq!(project, "proj-xyz");
    }

    #[test]
    fn parse_account_info_detects_paid_tier_from_allowed_tiers_robust() {
        let data = serde_json::json!({
            "cloudaicompanionProject": "p",
            "allowedTiers": [ { "id": "standard-tier", "isDefault": true } ],
        });
        let (_, tier) = parse_account_info(&data);
        assert_eq!(tier, AccountTier::Paid);
    }

    #[test]
    fn parse_account_info_keeps_free_for_legacy_and_free_tiers_robust() {
        for id in ["legacy-tier", "free-tier", "zero-tier"] {
            let data = serde_json::json!({
                "cloudaicompanionProject": "p",
                "allowedTiers": [ { "id": id, "isDefault": true } ],
            });
            let (_, tier) = parse_account_info(&data);
            assert_eq!(tier, AccountTier::Free, "tier id {id} should stay Free");
        }
    }

    #[test]
    fn parse_account_info_detects_paid_from_paid_tier_field_robust() {
        let data = serde_json::json!({
            "cloudaicompanionProject": "p",
            "paidTier": { "id": "ai-pro" },
        });
        let (_, tier) = parse_account_info(&data);
        assert_eq!(tier, AccountTier::Paid);
    }

    #[test]
    fn provider_lists_gemini_and_claude_models_normal() {
        let models = AntigravityOAuthProvider::antigravity_models();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"gemini-3.5-flash"));
        assert!(ids.contains(&"gemini-claude-sonnet-4-5-thinking"));
        // Subscription-covered: all costs zero.
        assert!(models.iter().all(|m| m.input_cost == Some(0.0)));
    }
}
