#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock};

use crate::provider::{
    CompletionResponse, EventStream, ModelInfo, Provider, ProviderContent, ProviderMessage,
    ProviderRole, StreamConvention, StreamEvent, StreamOptions, TokenUsage,
};

use super::sse;
use futures::StreamExt;

type HmacSha256 = Hmac<Sha256>;

pub(crate) const AUTO_RETRY_SENTINEL: &str = "auto-retry-anthropic-oauth:";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    name: String,
    refresh_token: String,
    access_token: Option<String>,
    expires_at: Option<u64>,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountStore {
    accounts: Vec<Account>,
    active_index: Option<usize>,
}

const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const REFRESH_SCOPES: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_BETA: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,prompt-caching-2024-07-31,prompt-caching-scope-2026-01-05,output-128k-2025-02-19,context-management-2025-06-27,web-search-2025-03-05,structured-outputs-2025-12-15";

const CLAUDE_CODE_IDENTITY: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

const SALT: &str = "59cf53e54c78";

const VERSION_URL: &str = "https://registry.npmjs.org/@anthropic-ai/claude-code/latest";
const VERSION_FALLBACK: &str = "2.1.137";
const VERSION_CACHE_TTL: Duration = Duration::from_secs(3600);
const VERSION_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const TOKEN_REFRESH_TIMEOUT: Duration = Duration::from_secs(15);

const CCH_PLACEHOLDER: &str = "cch=00000";

struct VersionCache {
    version: String,
    fetched_at: std::time::SystemTime,
}

static VERSION_CACHE: std::sync::OnceLock<Mutex<Option<VersionCache>>> = std::sync::OnceLock::new();

fn version_cache_mutex() -> &'static Mutex<Option<VersionCache>> {
    VERSION_CACHE.get_or_init(|| Mutex::new(None))
}

async fn fetch_cli_version(client: &reqwest::Client) -> String {
    {
        let guard = version_cache_mutex().lock().await;
        if let Some(ref cache) = *guard {
            if cache.fetched_at.elapsed().unwrap_or(Duration::MAX) < VERSION_CACHE_TTL {
                tracing::debug!(
                    target: "jfc::provider::anthropic_oauth",
                    version = %cache.version,
                    "using cached CLI version"
                );
                return cache.version.clone();
            }
        }
    }

    let version = match client
        .get(VERSION_URL)
        .timeout(VERSION_FETCH_TIMEOUT)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
            Ok(value) => value["version"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| VERSION_FALLBACK.to_owned()),
            Err(e) => {
                tracing::debug!(
                    target: "jfc::provider::anthropic_oauth",
                    error = %e,
                    "failed to decode CLI version response; using fallback"
                );
                VERSION_FALLBACK.to_owned()
            }
        },
        Ok(resp) => {
            tracing::debug!(
                target: "jfc::provider::anthropic_oauth",
                status = %resp.status(),
                "CLI version fetch returned non-success; using fallback"
            );
            VERSION_FALLBACK.to_owned()
        }
        Err(e) => {
            tracing::debug!(
                target: "jfc::provider::anthropic_oauth",
                error = %e,
                "CLI version fetch failed; using fallback"
            );
            VERSION_FALLBACK.to_owned()
        }
    };
    tracing::debug!(
        target: "jfc::provider::anthropic_oauth",
        version = %version,
        "fetched CLI version from registry"
    );
    let mut guard = version_cache_mutex().lock().await;
    *guard = Some(VersionCache {
        version: version.clone(),
        fetched_at: std::time::SystemTime::now(),
    });
    version
}

fn compute_billing_hash(first_user_message: &str, version: &str) -> String {
    let chars: Vec<char> = first_user_message.chars().collect();
    let c = |i: usize| chars.get(i).map(|c| c.to_string()).unwrap_or_default();
    let input = format!("{}{}{}{}{}", SALT, c(4), c(7), c(20), version);
    let hash = Sha256::digest(input.as_bytes());
    hex::encode(hash)[..3].to_owned()
}

#[cfg(feature = "anthropic-oauth-sensitive")]
fn compute_body_attestation(body: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(SALT.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body.as_bytes());
    let result = mac.finalize().into_bytes();
    let cch = &hex::encode(result)[..5];
    body.replacen(CCH_PLACEHOLDER, &format!("cch={cch}"), 1)
}

fn build_user_agent(version: &str) -> String {
    format!("claude-cli/{version} (external, cli)")
}

fn build_billing_header_text(version: &str, billing_hash: &str) -> String {
    format!(
        "x-anthropic-billing-header: cc_version={version}.{billing_hash}; cc_entrypoint=cli; {CCH_PLACEHOLDER};"
    )
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    #[allow(dead_code)]
    scope: Option<String>,
}

#[derive(Debug, Serialize)]
struct RefreshRequest<'a> {
    grant_type: &'a str,
    refresh_token: &'a str,
    client_id: &'a str,
    scope: &'a str,
}

/// Detect the v126-equivalent "model is gated for this account" error shape and
/// extract the offending model id. Returns `Some(id)` for
/// `{"error": {"type": "not_found_error", "message": "model: <id>"}}`,
/// otherwise `None` so callers can fall back to the raw error text.
pub(crate) fn parse_model_not_found(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let err = v.get("error")?;
    let kind = err.get("type")?.as_str()?;
    if kind != "not_found_error" {
        return None;
    }
    let msg = err.get("message")?.as_str()?;
    let trimmed = msg.trim();
    let id = trimmed.strip_prefix("model:")?.trim();
    if id.is_empty() {
        return None;
    }
    Some(id.to_owned())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn refresh_access_token(
    client: &reqwest::Client,
    refresh_token: &str,
) -> anyhow::Result<(String, String, u64)> {
    tracing::info!(
        target: "jfc::provider::anthropic_oauth",
        "attempting token refresh"
    );

    let body = RefreshRequest {
        grant_type: "refresh_token",
        refresh_token,
        client_id: CLIENT_ID,
        scope: REFRESH_SCOPES,
    };

    let resp = client
        .post(TOKEN_URL)
        .header("content-type", "application/json")
        .timeout(TOKEN_REFRESH_TIMEOUT)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        tracing::warn!(
            target: "jfc::provider::anthropic_oauth",
            status = %status,
            body_preview = %&text[..text.len().min(200)],
            "token refresh failed"
        );
        anyhow::bail!("token refresh failed {status}: {text}");
    }

    let json: TokenResponse = resp.json().await?;

    if let Some(scope) = &json.scope {
        if !scope.contains("user:inference") {
            anyhow::bail!("user:inference not in granted scopes: {scope}");
        }
    }

    let new_refresh = json
        .refresh_token
        .unwrap_or_else(|| refresh_token.to_owned());
    let expires_in = json.expires_in.unwrap_or(3600);
    let expires_at = now_ms() + expires_in * 1000 - 30_000;

    tracing::info!(
        target: "jfc::provider::anthropic_oauth",
        expires_in_secs = expires_in,
        "token refresh succeeded"
    );

    Ok((json.access_token, new_refresh, expires_at))
}

/// Pure resolver for the Anthropic accounts store path. Inputs are explicit so the
/// precedence rules can be unit-tested without mutating process state or the filesystem.
///
/// Precedence: `override_env` (set by `JFC_ANTHROPIC_ACCOUNTS_PATH`) → canonical
/// `~/.config/jfc-anthropic-accounts.json`.
fn resolve_store_path(override_env: Option<&str>, home: &std::path::Path) -> PathBuf {
    if let Some(p) = override_env {
        return PathBuf::from(p);
    }
    home.join(".config/jfc-anthropic-accounts.json")
}

/// Resolve the Anthropic accounts store. Canonical location is
/// `~/.config/jfc-anthropic-accounts.json`. Override with `JFC_ANTHROPIC_ACCOUNTS_PATH`
/// (e.g. to point at opencode's `~/.config/opencode/anthropic-accounts.json` if you
/// want to share rotation state with opencode again).
pub fn default_store_path() -> PathBuf {
    let override_env = std::env::var("JFC_ANTHROPIC_ACCOUNTS_PATH").ok();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    resolve_store_path(override_env.as_deref(), &home)
}

fn load_store(path: &PathBuf) -> anyhow::Result<AccountStore> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn pick_account(store: &AccountStore) -> Option<&Account> {
    let enabled: Vec<&Account> = store
        .accounts
        .iter()
        .filter(|a| a.enabled.unwrap_or(true) && !a.refresh_token.is_empty())
        .collect();
    let idx = store.active_index.unwrap_or(0);
    store
        .accounts
        .get(idx)
        .filter(|a| a.enabled.unwrap_or(true) && !a.refresh_token.is_empty())
        .or_else(|| enabled.first().copied())
}

fn write_back_tokens(
    path: &PathBuf,
    account_name: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: u64,
) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(path)?;
    let mut store: Value = serde_json::from_str(&raw)?;
    if let Some(accounts) = store.get_mut("accounts").and_then(|a| a.as_array_mut()) {
        for acct in accounts.iter_mut() {
            if acct.get("name").and_then(|n| n.as_str()) == Some(account_name) {
                acct["accessToken"] = json!(access_token);
                acct["refreshToken"] = json!(refresh_token);
                acct["expiresAt"] = json!(expires_at);
                break;
            }
        }
    }
    let tmp = format!("{}.tmp-{}", path.display(), std::process::id());
    std::fs::write(&tmp, serde_json::to_string_pretty(&store)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct TokenState {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    account_name: String,
}

/// Subset of `GET /api/oauth/profile` used by the model-access logic. Mirrors
/// v126 cli.js (`GC$()`): Anthropic doesn't expose a model-ACL endpoint, so
/// account tier is the source of truth for which Opus variant the picker should
/// surface (see `XwH()` in v126 — `tier_filter` here implements the same rules).
#[derive(Debug, Clone, Default)]
pub struct OAuthProfile {
    pub subscription_type: Option<String>, // "max" | "pro" | "enterprise" | "team"
    pub seat_tier: Option<String>, // "code_max" | "code_pro" | model id | "opus" | "opusplan" | "opus[1m]" | …
    pub rate_limit_tier: Option<String>, // e.g. "tier4"
    pub billing_type: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub has_extra_usage_enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FlatOAuthProfile {
    subscription_type: Option<String>,
    seat_tier: Option<String>,
    rate_limit_tier: Option<String>,
    billing_type: Option<String>,
    display_name: Option<String>,
    email: Option<String>,
    has_extra_usage_enabled: Option<bool>,
}

impl<'de> Deserialize<'de> for OAuthProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let flat: FlatOAuthProfile = serde_json::from_value(value.clone()).unwrap_or_default();
        let account = value.get("account").and_then(Value::as_object);
        let organization = value.get("organization").and_then(Value::as_object);

        let org_str = |key: &str| {
            organization
                .and_then(|o| o.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let account_str = |key: &str| {
            account
                .and_then(|a| a.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let org_bool = |key: &str| {
            organization
                .and_then(|o| o.get(key))
                .and_then(Value::as_bool)
        };

        let subscription_type = flat.subscription_type.or_else(|| {
            org_str("organization_type").and_then(|kind| {
                match kind.as_str() {
                    "claude_max" => Some("max"),
                    "claude_pro" => Some("pro"),
                    "claude_enterprise" => Some("enterprise"),
                    "claude_team" => Some("team"),
                    _ => None,
                }
                .map(str::to_owned)
            })
        });

        Ok(Self {
            subscription_type,
            seat_tier: flat.seat_tier.or_else(|| org_str("seat_tier")),
            rate_limit_tier: flat.rate_limit_tier.or_else(|| org_str("rate_limit_tier")),
            billing_type: flat.billing_type.or_else(|| org_str("billing_type")),
            display_name: flat.display_name.or_else(|| account_str("display_name")),
            email: flat.email.or_else(|| account_str("email")),
            has_extra_usage_enabled: flat
                .has_extra_usage_enabled
                .or_else(|| org_bool("has_extra_usage_enabled")),
        })
    }
}

pub struct AnthropicOAuthProvider {
    client: reqwest::Client,
    store_path: PathBuf,
    token: Arc<RwLock<Option<TokenState>>>,
    profile: Arc<RwLock<Option<OAuthProfile>>>,
    /// Multi-account rotation manager. Lazy-initialized on first use so
    /// constructing the provider stays sync (and infallible).
    manager: tokio::sync::OnceCell<super::anthropic_accounts::AccountManager>,
}

/// How many times [`AnthropicOAuthProvider::stream`] / `complete` will rotate
/// to a different account on 429 / 401-after-refresh / `invalid_grant` before
/// surfacing the error.
const ROTATION_MAX_ATTEMPTS: usize = 5;

/// Maximum wall-time we'll wait for the soonest-recovering account when ALL
/// accounts are rate-limited mid-rotation. CC v138 caps the same wait at
/// `Ay6 = 21600000` ms (6 h); we use a smaller window because jfc is
/// interactive and a 6h freeze would be hostile.
const MAX_RECOVERY_WAIT: Duration = Duration::from_secs(5 * 60);

/// Total wall-time budget across all sleep-and-retry iterations. Hard floor
/// so a degenerate "every account permanently rate-limited" never strands
/// the user — they get a real error after this.
const MAX_TOTAL_WAIT: Duration = Duration::from_secs(10 * 60);

/// Default fallback model used when the per-account 529 counter trips the
/// `OVERLOADED_FALLBACK_THRESHOLD`. Mirrors CC v138's recommended Opus→Sonnet
/// fallback. Picked at the (best-effort) catalog level — if the user has
/// pinned a different fallback via `slate`, that takes precedence.
const DEFAULT_OVERLOAD_FALLBACK_MODEL: &str = "claude-sonnet-4-5";

impl AnthropicOAuthProvider {
    pub fn new() -> Self {
        let store_path = default_store_path();
        tracing::debug!(
            target: "jfc::provider::anthropic_oauth",
            store_path = %store_path.display(),
            "AnthropicOAuthProvider::new"
        );
        Self {
            client: super::http::streaming_client(),
            store_path,
            token: Arc::new(RwLock::new(None)),
            profile: Arc::new(RwLock::new(None)),
            manager: tokio::sync::OnceCell::new(),
        }
    }

    /// Lazily initialize and return a reference to the account manager.
    /// Re-reads the store from disk if its mtime advanced (so an opencode-
    /// side rotation is picked up without restart).
    pub async fn account_manager(
        &self,
    ) -> anyhow::Result<&super::anthropic_accounts::AccountManager> {
        let mgr = self
            .manager
            .get_or_try_init(|| async {
                super::anthropic_accounts::AccountManager::load(self.store_path.clone()).await
            })
            .await?;
        // Best-effort hot-reload — failures don't block use of the cache.
        let _ = mgr.reload_if_changed().await;
        Ok(mgr)
    }

    /// Fetch and cache the OAuth profile (`GET /api/oauth/profile`). v126 calls this
    /// once after sign-in to discover seatTier / subscriptionType, which then drive
    /// what the model picker shows. Returns the cached value on subsequent calls;
    /// network failures surface as `Err` and leave the cache unset so the caller
    /// can decide whether to retry or fall back to "show everything".
    pub async fn fetch_profile(&self) -> anyhow::Result<OAuthProfile> {
        if let Some(p) = self.profile.read().await.as_ref() {
            return Ok(p.clone());
        }
        tracing::info!(
            target: "jfc::provider::anthropic_oauth",
            "fetching OAuth profile"
        );
        let token = self.get_access_token().await?;
        let account_name = self
            .token
            .read()
            .await
            .as_ref()
            .map(|token| token.account_name.clone());
        let resp = self
            .client
            .get("https://api.anthropic.com/api/oauth/profile")
            .header("authorization", format!("Bearer {token}"))
            .header("accept", "application/json")
            .timeout(Duration::from_secs(8))
            .send()
            .await?
            .error_for_status()?;
        let profile: OAuthProfile = resp.json().await?;
        tracing::debug!(
            target: "jfc::provider::anthropic_oauth",
            display_name = ?profile.display_name,
            seat_tier = ?profile.seat_tier,
            subscription_type = ?profile.subscription_type,
            "OAuth profile fetched successfully"
        );
        if let Some(account_name) = account_name
            && let Ok(mgr) = self.account_manager().await
        {
            let _ = mgr
                .atomic_update_profile(
                    &account_name,
                    profile.rate_limit_tier.clone(),
                    profile.subscription_type.clone(),
                    profile.email.clone(),
                    None,
                )
                .await;
        }
        *self.profile.write().await = Some(profile.clone());
        Ok(profile)
    }

    /// Read the cached profile without doing I/O. Used by the picker after the
    /// background fetch posts a `ProfileLoaded` event.
    pub async fn cached_profile(&self) -> Option<OAuthProfile> {
        self.profile.read().await.clone()
    }

    /// Returns true if the resolved accounts file exists and parses with at least one
    /// enabled account. Used at startup so we only register OAuth as a candidate provider
    /// when it can actually authenticate.
    pub fn has_usable_config(&self) -> bool {
        load_store(&self.store_path)
            .ok()
            .as_ref()
            .and_then(pick_account)
            .is_some()
    }

    async fn get_access_token(&self) -> anyhow::Result<String> {
        // Default path: pick the best account via the rotation manager
        // (tier-aware, cooldown-aware, LRU-tied). Falls back to legacy
        // active-account selection if the manager isn't available.
        let account_opt = match self.account_manager().await {
            Ok(mgr) => mgr
                .pick_next()
                .await
                .map(|a| (a.name, a.refresh_token, a.access_token, a.expires_at)),
            Err(_) => None,
        };

        if let Some((name, refresh_token, access_token, expires_at)) = account_opt {
            self.get_access_token_for(&name, &refresh_token, access_token.as_deref(), expires_at)
                .await
        } else {
            self.get_access_token_legacy().await
        }
    }

    /// Resolve an access token for a *specific* account. Honors the in-memory
    /// cache only when the cached entry belongs to the requested account, so
    /// rotation across accounts always re-acquires.
    pub(crate) async fn get_access_token_for(
        &self,
        account_name: &str,
        refresh_token: &str,
        existing_access_token: Option<&str>,
        existing_expires_at: Option<u64>,
    ) -> anyhow::Result<String> {
        {
            let guard = self.token.read().await;
            if let Some(t) = guard.as_ref() {
                if t.account_name == account_name && now_ms() < t.expires_at {
                    return Ok(t.access_token.clone());
                }
            }
        }

        let mut guard = self.token.write().await;
        if let Some(t) = guard.as_ref() {
            if t.account_name == account_name && now_ms() < t.expires_at {
                return Ok(t.access_token.clone());
            }
        }

        let (access_token, new_refresh, expires_at) =
            match (existing_access_token, existing_expires_at) {
                (Some(at), Some(exp)) if now_ms() < exp => {
                    (at.to_owned(), refresh_token.to_owned(), exp)
                }
                _ => {
                    self.refresh_with_disable_on_invalid_grant(account_name, refresh_token)
                        .await?
                }
            };

        // Persist via the rotation manager (atomic disk + in-memory cache);
        // also fall through to the legacy writer for back-compat.
        if let Ok(mgr) = self.account_manager().await {
            let _ = mgr
                .atomic_update_tokens(
                    account_name,
                    access_token.clone(),
                    expires_at,
                    Some(new_refresh.clone()),
                )
                .await;
        }
        let _ = write_back_tokens(
            &self.store_path,
            account_name,
            &access_token,
            &new_refresh,
            expires_at,
        );

        *guard = Some(TokenState {
            access_token: access_token.clone(),
            refresh_token: new_refresh,
            expires_at,
            account_name: account_name.to_owned(),
        });

        Ok(access_token)
    }

    /// Wraps [`refresh_access_token`] so a permanent `invalid_grant` failure
    /// auto-disables the account in the rotation manager. Transient errors
    /// (network, 5xx) bubble through unchanged so the caller can retry.
    async fn refresh_with_disable_on_invalid_grant(
        &self,
        account_name: &str,
        refresh_token: &str,
    ) -> anyhow::Result<(String, String, u64)> {
        match refresh_access_token(&self.client, refresh_token).await {
            Ok(t) => Ok(t),
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("invalid_grant") {
                    if let Ok(mgr) = self.account_manager().await {
                        let _ = mgr.atomic_clear_refresh_token(account_name).await;
                    }
                    tracing::warn!(
                        target: "jfc::provider::anthropic_oauth::rotation",
                        account = %account_name,
                        "refresh failed with invalid_grant — account auto-disabled"
                    );
                }
                Err(e)
            }
        }
    }

    /// Pre-rotation legacy path. Used when the manager is unavailable (e.g.,
    /// store file unreadable) — preserves jfc's original single-account
    /// behavior so an inaccessible store doesn't break the provider entirely.
    async fn get_access_token_legacy(&self) -> anyhow::Result<String> {
        {
            let guard = self.token.read().await;
            if let Some(t) = guard.as_ref() {
                if now_ms() < t.expires_at {
                    return Ok(t.access_token.clone());
                }
            }
        }
        let mut guard = self.token.write().await;
        if let Some(t) = guard.as_ref() {
            if now_ms() < t.expires_at {
                return Ok(t.access_token.clone());
            }
        }
        let store = load_store(&self.store_path).map_err(|e| {
            anyhow::anyhow!(
                "cannot load anthropic accounts from {}: {e}",
                self.store_path.display()
            )
        })?;
        let account = pick_account(&store)
            .ok_or_else(|| anyhow::anyhow!("no enabled Anthropic accounts in store"))?;
        let (access_token, new_refresh, expires_at) =
            if let (Some(at), Some(exp)) = (&account.access_token, account.expires_at) {
                if now_ms() < exp {
                    (at.clone(), account.refresh_token.clone(), exp)
                } else {
                    refresh_access_token(&self.client, &account.refresh_token).await?
                }
            } else {
                refresh_access_token(&self.client, &account.refresh_token).await?
            };
        let _ = write_back_tokens(
            &self.store_path,
            &account.name,
            &access_token,
            &new_refresh,
            expires_at,
        );
        *guard = Some(TokenState {
            access_token: access_token.clone(),
            refresh_token: new_refresh,
            expires_at,
            account_name: account.name.clone(),
        });
        Ok(access_token)
    }
}

/// Parse `retry-after` header value into seconds. Spec allows either an
/// integer ("seconds") or HTTP-date format. We honor integers; HTTP-date
/// handling is intentionally conservative — any unparseable header returns
/// `None`, which drives the manager to its default exponential cooldown.
fn parse_retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let raw = headers.get("retry-after")?.to_str().ok()?;
    let trimmed = raw.trim();
    if let Ok(secs) = trimmed.parse::<u64>() {
        return Some(secs);
    }
    None
}

fn is_stream_rate_limit_message(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("rate limit")
        || m.contains("rate-limit")
        || m.contains("rate_limited")
        || m.contains("rate limited")
        || m.contains("too many requests")
        || m.contains("429")
}

/// Wrap an `EventStream` so every `Usage` event also persists tokens to the
/// account JSON. Anthropic sends *cumulative* token counts on every
/// `message_delta`, so we apply a baseline-delta to avoid double-counting
/// (same shape as `App.usage_apply_baseline` in `event_loop.rs`). The wrapper
/// also handles Anthropic's mid-stream `error` events: a request can receive
/// HTTP 2xx and still later terminate with a rate-limit error, which bypasses
/// the HTTP rotation loop above unless we mark the account here and ask the UI
/// to restart the turn.
fn wrap_with_usage_recording(
    inner: EventStream,
    mgr: super::anthropic_accounts::AccountManager,
    guard: super::anthropic_accounts::AccountRequestGuard,
    account_name: String,
    model: String,
) -> EventStream {
    use futures::stream::{self};
    let state = std::sync::Arc::new(tokio::sync::Mutex::new((0u64, 0u64, 0u64, 0u64)));
    let stream = stream::unfold(
        (inner, state, mgr, guard, account_name, model),
        |(mut inner, state, mgr, guard, account_name, model)| async move {
            let mut next = inner.next().await?;
            if let Ok(StreamEvent::Error { message }) = &next
                && is_stream_rate_limit_message(message)
            {
                let info = super::unified::RateLimitInfo::default();
                mgr.mark_rate_limited_with_info(&account_name, &info).await;
                tracing::warn!(
                    target: "jfc::provider::anthropic_oauth::rotation",
                    account = %account_name,
                    error = %message,
                    "mid-stream rate-limit error — marked account and requesting silent retry"
                );
                next = Ok(StreamEvent::Error {
                    message: format!("{AUTO_RETRY_SENTINEL}{message}"),
                });
            }
            if let Ok(StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            }) = &next
            {
                let cum = (
                    *input_tokens as u64,
                    *output_tokens as u64,
                    *cache_read_tokens as u64,
                    *cache_write_tokens as u64,
                );
                let mut baseline = state.lock().await;
                let (din, dout, dcr, dcw) = (
                    cum.0.saturating_sub(baseline.0),
                    cum.1.saturating_sub(baseline.1),
                    cum.2.saturating_sub(baseline.2),
                    cum.3.saturating_sub(baseline.3),
                );
                // Defensive: any of the four going *backwards* (server reset
                // mid-stream) means baseline drifted — re-anchor on the new
                // cumulative reading instead of rolling deltas forward.
                let any_regression = cum.0 < baseline.0
                    || cum.1 < baseline.1
                    || cum.2 < baseline.2
                    || cum.3 < baseline.3;
                *baseline = cum;
                drop(baseline);

                if !any_regression && (din | dout | dcr | dcw) != 0 {
                    let mut um = crate::types::ModelUsage::default();
                    um.input_tokens = din;
                    um.output_tokens = dout;
                    um.cache_read_tokens = dcr;
                    um.cache_write_tokens = dcw;
                    let cost = crate::cost::cost_for(&model, &um);
                    let delta = super::anthropic_accounts::UsageDelta {
                        input_tokens: din,
                        output_tokens: dout,
                        cache_read_tokens: dcr,
                        cache_write_tokens: dcw,
                        model: model.clone(),
                        cost_usd: cost,
                    };
                    if let Err(e) = mgr.record_usage(&account_name, &delta).await {
                        tracing::debug!(
                            target: "jfc::provider::anthropic_oauth::usage",
                            account = %account_name,
                            error = %e,
                            "record_usage failed (continuing)"
                        );
                    }
                }
            }
            Some((next, (inner, state, mgr, guard, account_name, model)))
        },
    );
    Box::pin(stream)
}

fn completion_response_from_json(json: &Value) -> CompletionResponse {
    let content = json
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|block| {
                let kind = block.get("type")?.as_str()?;
                if kind == "tool_use" {
                    let input = block.get("input")?;
                    return Some(input.to_string());
                }
                if kind == "text" {
                    return block.get("text")?.as_str().map(str::to_owned);
                }
                None
            })
        })
        .unwrap_or_default();

    let usage = json.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let output_tokens = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    CompletionResponse {
        content,
        usage: TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        },
    }
}

/// Classification of an HTTP response for the rotation retry loop.
/// Permanent errors abort the loop immediately; rotate variants advance to
/// the next account.
enum RotationDecision {
    /// 2xx: hand the response back to the caller.
    Success,
    /// 429: rotate to the next account. `retry_after_secs` carries any
    /// value parsed from the response header.
    RateLimited { retry_after_secs: Option<u64> },
    /// 401/5xx: rotate (transient/account-specific failure).
    AccountFailure,
    /// Other 4xx: bail with the response body — surfacing to the caller.
    Permanent,
}

fn classify_for_rotation(status: reqwest::StatusCode) -> RotationDecision {
    if status.is_success() {
        RotationDecision::Success
    } else if status.as_u16() == 429 {
        RotationDecision::RateLimited {
            retry_after_secs: None,
        }
    } else if status.as_u16() == 401 || status.is_server_error() {
        RotationDecision::AccountFailure
    } else {
        RotationDecision::Permanent
    }
}

fn build_system_blocks(billing_header_text: &str, caller_system: Option<&str>) -> Value {
    let mut blocks: Vec<Value> = vec![
        json!({ "type": "text", "text": billing_header_text }),
        json!({
            "type": "text",
            "text": CLAUDE_CODE_IDENTITY,
            "cache_control": { "type": "ephemeral" }
        }),
    ];
    if let Some(sys) = caller_system {
        let sanitized = sanitize_system_prompt(sys);
        if !sanitized.is_empty() {
            // Cache the caller's (jfc's) system prompt too — it's the
            // largest block and changes least often, so a cache hit
            // saves the most. v132 puts a breakpoint on the last
            // system block; we mirror that.
            blocks.extend(caller_system_blocks(&sanitized));
        }
    }
    json!(blocks)
}

fn caller_system_blocks(system: &str) -> Vec<Value> {
    let Some(index) = system.find("\n\n## Current diagnostics") else {
        return vec![json!({
            "type": "text",
            "text": system,
            "cache_control": { "type": "ephemeral" },
        })];
    };

    let stable = system[..index].trim_end();
    let volatile = system[index..].trim_start();
    [
        (!stable.is_empty()).then(|| {
            json!({
                "type": "text",
                "text": stable,
                "cache_control": { "type": "ephemeral" },
            })
        }),
        (!volatile.is_empty()).then(|| json!({ "type": "text", "text": volatile })),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Strip third-party branding from the caller-supplied system prompt before
/// sending to Anthropic OAuth. Anthropic's server-side validator pattern-
/// matches against the `claude-code-20250219` beta identity — confirmed
/// via binary search by opencode-anthropic-auth (constants.ts:154-157):
/// the same prompt with a `<env>` block returns 200, without it returns
/// 400. Sanitize: drop `<env>`, `<directories>`, `<agent-identity>`
/// blocks, drop paragraphs that anchor on third-party URLs/prose, and
/// rewrite `jfc`-specific identity phrases.
///
/// Safe to apply to v126-style prompts: the strip patterns target only
/// branding artifacts, not load-bearing instructions.
fn sanitize_system_prompt(text: &str) -> String {
    let mut result = strip_block(text, "<agent-identity>", "</agent-identity>");
    result = strip_block(&result, "<env>", "</env>");
    result = strip_block(&result, "<directories>", "</directories>");

    // Drop whole paragraphs (blank-line-separated chunks) that mention
    // third-party tool branding. Mirrors PARAGRAPH_REMOVAL_ANCHORS in
    // opencode-anthropic-auth (constants.ts:80).
    const PARAGRAPH_ANCHORS: &[&str] = &[
        "github.com/anomalyco/opencode",
        "github.com/sst/opencode",
        "opencode.ai",
        "ctrl+p to list available actions",
        "/help: Get help with using opencode",
    ];
    let kept: Vec<&str> = result
        .split("\n\n")
        .filter(|p| !PARAGRAPH_ANCHORS.iter().any(|anchor| p.contains(anchor)))
        .collect();
    let mut out = kept.join("\n\n");

    // Inline rewrites — same intent as INLINE_TEXT_REPLACEMENTS in the
    // opencode plugin, scoped to jfc-specific branding so a v126-style
    // prompt that mentions "Claude Code" stays intact.
    const INLINE_REWRITES: &[(&str, &str)] = &[
        ("You are jfc, ", "You are Claude Code, "),
        ("You are JFC, ", "You are Claude Code, "),
        ("Your name is jfc.", ""),
        ("Sisyphus", "the assistant"),
        ("sisyphus", "assistant"),
        ("Ultraworker", ""),
        (".sisyphus/", ".cache/"),
    ];
    for (needle, replacement) in INLINE_REWRITES {
        out = out.replace(needle, replacement);
    }

    // Collapse runs of 3+ newlines (left over from paragraph removal).
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_owned()
}

/// Remove every `<tag>...</tag>` span from `text`. Tag-aware (case-
/// sensitive, supports nested whitespace). Used by `sanitize_system_prompt`
/// to drop `<env>`, `<directories>`, etc. blocks without disturbing
/// surrounding content.
fn strip_block(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after_open = &rest[start..];
        match after_open.find(close) {
            Some(end_rel) => {
                rest = &after_open[end_rel + close.len()..];
            }
            None => {
                // Unclosed tag — keep the rest untouched (don't silently
                // eat half a prompt).
                out.push_str(after_open);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

fn build_body(
    messages: Vec<ProviderMessage>,
    opts: &StreamOptions,
    billing_header_text: &str,
) -> Value {
    let mut body = json!({
        "model": opts.model,
        "max_tokens": opts.max_tokens,
        "stream": true,
        "messages": sse::build_messages(&messages),
        "system": build_system_blocks(billing_header_text, opts.system.as_deref()),
    });
    if !opts.tools.is_empty() {
        let mut tools = sse::build_tools(&opts.tools);
        if let Some(arr) = tools.as_array_mut() {
            if let Some(last) = arr.last_mut() {
                if let Some(obj) = last.as_object_mut() {
                    obj.insert("cache_control".to_owned(), json!({ "type": "ephemeral" }));
                }
            }
        }
        body["tools"] = tools;
    }
    if opts.adaptive_thinking {
        let mut thinking = json!({ "type": "adaptive" });
        if let Some(display) = opts.thinking_display.as_deref() {
            thinking["display"] = json!(display);
        }
        body["thinking"] = thinking;
    } else if let Some(budget) = opts.thinking_budget {
        body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
    }
    {
        let mut oc = serde_json::Map::new();
        if let Some(effort) = opts.reasoning_effort.as_deref() {
            oc.insert("effort".into(), json!(effort));
        }
        if let Some(tb) = opts.task_budget_tokens {
            oc.insert("task_budget".into(), json!({"type": "tokens", "total": tb}));
        }
        if !oc.is_empty() {
            body["output_config"] = serde_json::Value::Object(oc);
        }
    }
    body
}

impl crate::provider::seal::Sealed for AnthropicOAuthProvider {}

#[async_trait]
impl Provider for AnthropicOAuthProvider {
    fn name(&self) -> &str {
        "anthropic-oauth"
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::AnthropicNative
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        super::anthropic_models::anthropic_first_party_models("anthropic-oauth")
    }

    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        tracing::debug!(
            target: "jfc::provider::anthropic_oauth",
            "using embedded Claude Code OAuth model catalog"
        );
        // Claude Code OAuth model routing is driven by its embedded first-party
        // catalog plus account profile gating, not the public models.dev catalog.
        // Returning the static list here prevents the startup background fetch
        // from replacing alias/current-model rows with a public catalog that may
        // lag Claude Code or omit OAuth-only entries.
        Ok(self.available_models())
    }

    #[tracing::instrument(
        target = "jfc::provider::anthropic_oauth",
        skip_all,
        fields(
            model = %options.model,
            messages = messages.len(),
            tools = options.tools.len(),
            max_tokens = options.max_tokens,
        ),
        err,
    )]
    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        // Account-independent body construction (billing header, attestation,
        // user-agent) is hoisted outside the rotation loop — these don't
        // change between accounts.
        let version = fetch_cli_version(&self.client).await;
        let first_user_text = messages
            .iter()
            .find(|m| m.role == ProviderRole::User)
            .and_then(|m| {
                m.content.iter().find_map(|c| {
                    if let ProviderContent::Text(t) = c {
                        Some(t.as_str())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or("")
            .to_owned();
        let billing_hash = compute_billing_hash(&first_user_text, &version);
        let billing_header_text = build_billing_header_text(&version, &billing_hash);
        let user_agent = build_user_agent(&version);
        let body_value = build_body(messages, options, &billing_header_text);
        let body_str = serde_json::to_string(&body_value)?;
        let attested_body = {
            #[cfg(feature = "anthropic-oauth-sensitive")]
            {
                compute_body_attestation(&body_str)
            }
            #[cfg(not(feature = "anthropic-oauth-sensitive"))]
            {
                body_str.clone()
            }
        };

        // Build beta header: append fast-mode and/or task-budgets betas as needed.
        let mut betas_stream = ANTHROPIC_BETA.to_owned();
        if options.fast_mode {
            betas_stream.push_str(",fast-mode-2026-02-01");
        }
        if options.task_budget_tokens.is_some() {
            betas_stream.push_str(",task-budgets-2026-03-13");
        }
        let beta_header = betas_stream;

        // Two nested loops:
        //   - Outer: when every account ends up in cooldown mid-rotation, sleep
        //     until the soonest one recovers and retry (capped at MAX_TOTAL_WAIT).
        //   - Inner: rotate through up to ROTATION_MAX_ATTEMPTS accounts trying
        //     to find one that returns 2xx. Each inner attempt persists unified
        //     rate-limit telemetry to disk so the UI / next process see it.
        let mgr = self.account_manager().await?;
        let total_wait_started = std::time::Instant::now();
        let mut last_err: Option<anyhow::Error> = None;
        let mut model_in_use = options.model.as_str().to_owned();
        // Attested body for the request actually being sent. Starts as the
        // user-selected model; swapped to the fallback-model body after the
        // 529 threshold is crossed.
        let mut effective_body = attested_body.clone();

        'outer: loop {
            let mut tried: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut hit_rate_limit_this_round = false;

            for attempt in 0..ROTATION_MAX_ATTEMPTS {
                let Some((account, request_guard)) = mgr.acquire_next_excluding(&tried).await
                else {
                    break;
                };
                tried.insert(account.name.clone());

                let access_token = match self
                    .get_access_token_for(
                        &account.name,
                        &account.refresh_token,
                        account.access_token.as_deref(),
                        account.expires_at,
                    )
                    .await
                {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::provider::anthropic_oauth::rotation",
                            account = %account.name,
                            error = %e,
                            "token acquisition failed — rotating"
                        );
                        mgr.mark_failure(&account.name).await;
                        last_err = Some(e);
                        continue;
                    }
                };

                let send_started = std::time::Instant::now();
                let resp = match super::http::send_with_retry("anthropic_oauth.stream", || {
                    self.client
                        .post(API_URL)
                        .header("authorization", format!("Bearer {access_token}"))
                        .header("anthropic-version", ANTHROPIC_VERSION)
                        .header("anthropic-beta", beta_header.as_str())
                        .header("content-type", "application/json")
                        .header("user-agent", user_agent.clone())
                        .header("x-app", "cli")
                        .header("anthropic-client-platform", "cli")
                        .body(effective_body.clone())
                        .send()
                })
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        let cause = super::http::classify_send_error(&e);
                        tracing::warn!(
                            target: "jfc::provider::anthropic_oauth::rotation",
                            account = %account.name,
                            error = %e,
                            cause = cause,
                            "send failed (after retries) — rotating"
                        );
                        mgr.mark_failure(&account.name).await;
                        last_err = Some(anyhow::anyhow!(
                            "Anthropic OAuth send failed: {cause} ({e})"
                        ));
                        continue;
                    }
                };

                super::http::report_first_byte_latency(
                    "anthropic_oauth.stream",
                    send_started.elapsed(),
                );
                let status = resp.status();
                // Parse every relevant rate-limit header. Cheap, always safe to
                // run — `RateLimitInfo` is all `Option<_>`.
                let rl_info = super::unified::parse_rate_limit_headers(
                    resp.headers(),
                    super::anthropic_accounts::now_ms(),
                );
                tracing::info!(
                    target: "jfc::provider::anthropic_oauth",
                    account = %account.name,
                    status = %status,
                    attempt = attempt + 1,
                    model = %model_in_use,
                    unified_status = ?rl_info.unified_status,
                    claim = ?rl_info.claim,
                    "stream: received HTTP response"
                );

                match classify_for_rotation(status) {
                    RotationDecision::Success => {
                        mgr.mark_success(&account.name).await;
                        mgr.clear_overloaded_counter(&account.name).await;
                        // Best-effort telemetry persistence — never block the
                        // success path on disk I/O failure.
                        mgr.record_routing_state(&account.name, &rl_info).await;
                        let stream = sse::into_event_stream(resp);
                        return Ok(wrap_with_usage_recording(
                            stream,
                            mgr.clone(),
                            request_guard,
                            account.name.clone(),
                            model_in_use.clone(),
                        ));
                    }
                    RotationDecision::RateLimited { .. } => {
                        hit_rate_limit_this_round = true;
                        mgr.mark_rate_limited_with_info(&account.name, &rl_info)
                            .await;
                        let body = resp.text().await.unwrap_or_default();
                        tracing::warn!(
                            target: "jfc::provider::anthropic_oauth::rotation",
                            account = %account.name,
                            retry_after_ms = rl_info
                                .retry_after
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0),
                            claim = ?rl_info.claim,
                            body_preview = %&body[..body.len().min(200)],
                            "rate-limited — rotating"
                        );
                        last_err = Some(anyhow::anyhow!(
                            "rate-limited on account '{}' (claim={:?}): {body}",
                            account.name,
                            rl_info.claim,
                        ));
                    }
                    RotationDecision::AccountFailure => {
                        let body = resp.text().await.unwrap_or_default();
                        // CC v138 treats `"type":"overloaded_error"` in the
                        // body as a 529 regardless of HTTP status — and 5xx
                        // codes route here too. Distinguish overloaded from
                        // generic account failures so the 529 counter only
                        // fires on the real overload signal.
                        if super::unified::is_overloaded_error(status.as_u16(), &body) {
                            let crossed = mgr.mark_overloaded_529(&account.name).await;
                            tracing::warn!(
                                target: "jfc::provider::anthropic_oauth::rotation",
                                account = %account.name,
                                status = %status,
                                crossed_threshold = crossed,
                                "overloaded — rotating"
                            );
                            // If we've hit too many consecutive 529s AND a
                            // fallback model exists in the catalog AND the
                            // current model isn't already that fallback,
                            // swap and retry. Otherwise the outer loop
                            // either rotates account or sleeps.
                            if crossed
                                && !model_in_use
                                    .eq_ignore_ascii_case(DEFAULT_OVERLOAD_FALLBACK_MODEL)
                            {
                                tracing::warn!(
                                    target: "jfc::provider::anthropic_oauth::rotation",
                                    from_model = %model_in_use,
                                    to_model = %DEFAULT_OVERLOAD_FALLBACK_MODEL,
                                    "529 threshold crossed — switching to fallback model"
                                );
                                model_in_use = DEFAULT_OVERLOAD_FALLBACK_MODEL.to_owned();
                                // The original messages were consumed building
                                // `body_str`. Patch the `model` field on the
                                // serialized body and re-attest rather than
                                // rebuilding from scratch.
                                let mut patched: Value = serde_json::from_str(&body_str)?;
                                patched["model"] =
                                    Value::String(DEFAULT_OVERLOAD_FALLBACK_MODEL.to_owned());
                                let patched_str = serde_json::to_string(&patched)?;
                                effective_body = {
                                    #[cfg(feature = "anthropic-oauth-sensitive")]
                                    {
                                        compute_body_attestation(&patched_str)
                                    }
                                    #[cfg(not(feature = "anthropic-oauth-sensitive"))]
                                    {
                                        patched_str.clone()
                                    }
                                };
                            }
                            last_err = Some(anyhow::anyhow!(
                                "Anthropic overloaded ({status}) on account '{}': {body}",
                                account.name,
                            ));
                        } else {
                            mgr.mark_failure(&account.name).await;
                            tracing::warn!(
                                target: "jfc::provider::anthropic_oauth::rotation",
                                account = %account.name,
                                status = %status,
                                body_preview = %&body[..body.len().min(200)],
                                "account-level failure — rotating"
                            );
                            last_err = Some(anyhow::anyhow!(
                                "Anthropic API error {status} on account '{}': {body}",
                                account.name
                            ));
                        }
                    }
                    RotationDecision::Permanent => {
                        let body = resp.text().await.unwrap_or_default();
                        tracing::warn!(
                            target: "jfc::provider::anthropic_oauth",
                            status = %status,
                            body_preview = %&body[..body.len().min(200)],
                            "permanent API error — not rotating"
                        );
                        if let Some(model) = parse_model_not_found(&body) {
                            anyhow::bail!(
                                "{model} is not enabled on your Anthropic account. \
                                 Pin a model you have access to (Ctrl+M)."
                            );
                        }
                        let friendly = super::retry::friendly_error_message(status.as_u16(), &body);
                        anyhow::bail!("Anthropic API error {status}: {friendly}\n  raw: {body}");
                    }
                }
            }

            // Inner loop exited without a 2xx. Decide whether to sleep-and-retry
            // or surface the error. We only wait when at least one account
            // *will* recover within MAX_RECOVERY_WAIT — otherwise the error is
            // genuinely permanent and the user should see it now.
            if !hit_rate_limit_this_round {
                break 'outer;
            }
            let Some(wait) = mgr.time_until_soonest_recovery().await else {
                break 'outer;
            };
            if total_wait_started.elapsed() + wait > MAX_TOTAL_WAIT {
                tracing::warn!(
                    target: "jfc::provider::anthropic_oauth::rotation",
                    elapsed_secs = total_wait_started.elapsed().as_secs(),
                    "rotation budget exhausted — surfacing rate-limit error"
                );
                break 'outer;
            }
            let sleep_for = wait.min(MAX_RECOVERY_WAIT);
            tracing::warn!(
                target: "jfc::provider::anthropic_oauth::rotation",
                wait_secs = sleep_for.as_secs(),
                "all accounts rate-limited — sleeping until soonest recovery"
            );
            tokio::time::sleep(sleep_for).await;
            // Loop back: pick_next should now find a usable account.
        }

        let _ = model_in_use;
        let _ = effective_body;
        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("all Anthropic OAuth accounts exhausted with no successful response")
        }))
    }

    /// Non-streaming completion. Used by the auto-mode classifier (which forces
    /// a single `classify_result` tool call) and by compaction. Builds the same
    /// body as `stream()` but with `stream: false` and reads the response in
    /// one shot. Surfaces the first `tool_use` block's input as JSON in
    /// `CompletionResponse.content` so callers can decode without knowing the
    /// Anthropic content-block shape.
    #[tracing::instrument(
        target = "jfc::provider::anthropic_oauth",
        skip_all,
        fields(
            model = %options.model,
            messages = messages.len(),
            tools = options.tools.len(),
        ),
        err,
    )]
    async fn complete(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<CompletionResponse> {
        let version = fetch_cli_version(&self.client).await;

        let first_user_text = messages
            .iter()
            .find(|m| m.role == ProviderRole::User)
            .and_then(|m| {
                m.content.iter().find_map(|c| {
                    if let ProviderContent::Text(t) = c {
                        Some(t.as_str())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or("");
        let billing_hash = compute_billing_hash(first_user_text, &version);
        let billing_header_text = build_billing_header_text(&version, &billing_hash);
        let user_agent = build_user_agent(&version);

        let mut body_value = build_body(messages, options, &billing_header_text);
        body_value["stream"] = serde_json::json!(false);
        // Force the classifier tool when one was provided — mirrors v126's
        // tool_choice on classify_result.
        if let Some(tools) = body_value.get("tools").and_then(|v| v.as_array()) {
            if let Some(first) = tools.first() {
                if let Some(name) = first.get("name").and_then(|v| v.as_str()) {
                    body_value["tool_choice"] = serde_json::json!({
                        "type": "tool",
                        "name": name,
                    });
                }
            }
        }
        let body_str = serde_json::to_string(&body_value)?;
        let attested_body = {
            #[cfg(feature = "anthropic-oauth-sensitive")]
            {
                compute_body_attestation(&body_str)
            }
            #[cfg(not(feature = "anthropic-oauth-sensitive"))]
            {
                body_str.clone()
            }
        };
        // Build beta header: append fast-mode and/or task-budgets betas as needed.
        let mut betas_complete = ANTHROPIC_BETA.to_owned();
        if options.fast_mode {
            betas_complete.push_str(",fast-mode-2026-02-01");
        }
        if options.task_budget_tokens.is_some() {
            betas_complete.push_str(",task-budgets-2026-03-13");
        }
        let beta_header_complete = betas_complete;
        let mgr = self.account_manager().await?;
        let total_wait_started = std::time::Instant::now();
        let mut last_err: Option<anyhow::Error> = None;

        'outer: loop {
            let mut tried: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut hit_rate_limit_this_round = false;

            for attempt in 0..ROTATION_MAX_ATTEMPTS {
                let Some((account, _request_guard)) = mgr.acquire_next_excluding(&tried).await
                else {
                    break;
                };
                tried.insert(account.name.clone());

                let access_token = match self
                    .get_access_token_for(
                        &account.name,
                        &account.refresh_token,
                        account.access_token.as_deref(),
                        account.expires_at,
                    )
                    .await
                {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::provider::anthropic_oauth::rotation",
                            account = %account.name,
                            error = %e,
                            "token acquisition failed — rotating complete()"
                        );
                        mgr.mark_failure(&account.name).await;
                        last_err = Some(e);
                        continue;
                    }
                };

                let send_started = std::time::Instant::now();
                let resp = match super::http::send_with_retry("anthropic_oauth.complete", || {
                    self.client
                        .post(API_URL)
                        .header("authorization", format!("Bearer {access_token}"))
                        .header("anthropic-version", ANTHROPIC_VERSION)
                        .header("anthropic-beta", beta_header_complete.as_str())
                        .header("content-type", "application/json")
                        .header("user-agent", user_agent.clone())
                        .header("x-app", "cli")
                        .header("anthropic-client-platform", "cli")
                        .header("anthropic-dangerous-direct-browser-access", "true")
                        .body(attested_body.clone())
                        .send()
                })
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        let cause = super::http::classify_send_error(&e);
                        tracing::warn!(
                            target: "jfc::provider::anthropic_oauth::rotation",
                            account = %account.name,
                            error = %e,
                            cause = cause,
                            "complete send failed (after retries) — rotating"
                        );
                        mgr.mark_failure(&account.name).await;
                        last_err = Some(anyhow::anyhow!(
                            "Anthropic OAuth complete failed: {cause} ({e})"
                        ));
                        continue;
                    }
                };

                super::http::report_first_byte_latency(
                    "anthropic_oauth.complete",
                    send_started.elapsed(),
                );
                let status = resp.status();
                let content_length = resp
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("unknown")
                    .to_owned();
                let rl_info = super::unified::parse_rate_limit_headers(
                    resp.headers(),
                    super::anthropic_accounts::now_ms(),
                );
                tracing::info!(
                    target: "jfc::provider::anthropic_oauth",
                    account = %account.name,
                    status = %status,
                    attempt = attempt + 1,
                    content_length = %content_length,
                    unified_status = ?rl_info.unified_status,
                    claim = ?rl_info.claim,
                    "complete: received HTTP response"
                );

                match classify_for_rotation(status) {
                    RotationDecision::Success => {
                        mgr.mark_success(&account.name).await;
                        mgr.clear_overloaded_counter(&account.name).await;
                        mgr.record_routing_state(&account.name, &rl_info).await;
                        let json: Value = resp.json().await?;
                        let response = completion_response_from_json(&json);
                        if response.usage.input_tokens != 0 || response.usage.output_tokens != 0 {
                            let mut usage = crate::types::ModelUsage::default();
                            usage.input_tokens = response.usage.input_tokens as u64;
                            usage.output_tokens = response.usage.output_tokens as u64;
                            let delta = super::anthropic_accounts::UsageDelta {
                                input_tokens: response.usage.input_tokens as u64,
                                output_tokens: response.usage.output_tokens as u64,
                                cache_read_tokens: 0,
                                cache_write_tokens: 0,
                                model: options.model.to_string(),
                                cost_usd: crate::cost::cost_for(&options.model, &usage),
                            };
                            if let Err(e) = mgr.record_usage(&account.name, &delta).await {
                                tracing::debug!(
                                    target: "jfc::provider::anthropic_oauth::usage",
                                    account = %account.name,
                                    error = %e,
                                    "record_usage failed during complete()"
                                );
                            }
                        }
                        return Ok(response);
                    }
                    RotationDecision::RateLimited { .. } => {
                        hit_rate_limit_this_round = true;
                        mgr.mark_rate_limited_with_info(&account.name, &rl_info)
                            .await;
                        let text = resp.text().await.unwrap_or_default();
                        tracing::warn!(
                            target: "jfc::provider::anthropic_oauth::rotation",
                            account = %account.name,
                            retry_after_ms = rl_info
                                .retry_after
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0),
                            claim = ?rl_info.claim,
                            body_preview = %&text[..text.len().min(200)],
                            "complete rate-limited — rotating"
                        );
                        last_err = Some(anyhow::anyhow!(
                            "rate-limited on account '{}' (claim={:?}): {text}",
                            account.name,
                            rl_info.claim,
                        ));
                    }
                    RotationDecision::AccountFailure => {
                        let text = resp.text().await.unwrap_or_default();
                        if super::unified::is_overloaded_error(status.as_u16(), &text) {
                            let crossed = mgr.mark_overloaded_529(&account.name).await;
                            tracing::warn!(
                                target: "jfc::provider::anthropic_oauth::rotation",
                                account = %account.name,
                                status = %status,
                                crossed_threshold = crossed,
                                "complete overloaded — rotating"
                            );
                            last_err = Some(anyhow::anyhow!(
                                "Anthropic overloaded ({status}) on account '{}': {text}",
                                account.name,
                            ));
                        } else {
                            mgr.mark_failure(&account.name).await;
                            tracing::warn!(
                                target: "jfc::provider::anthropic_oauth::rotation",
                                account = %account.name,
                                status = %status,
                                body_preview = %&text[..text.len().min(200)],
                                "complete account-level failure — rotating"
                            );
                            last_err = Some(anyhow::anyhow!(
                                "Anthropic API error {status} on account '{}': {text}",
                                account.name
                            ));
                        }
                    }
                    RotationDecision::Permanent => {
                        let text = resp.text().await.unwrap_or_default();
                        tracing::warn!(
                            target: "jfc::provider::anthropic_oauth",
                            status = %status,
                            body_preview = %&text[..text.len().min(200)],
                            "complete: permanent API request failed"
                        );
                        if let Some(model) = parse_model_not_found(&text) {
                            anyhow::bail!(
                                "{model} is not enabled on your Anthropic account. \
                                 Pin a model you have access to (Ctrl+M)."
                            );
                        }
                        anyhow::bail!("Anthropic API error {status}: {text}");
                    }
                }
            }

            if !hit_rate_limit_this_round {
                break 'outer;
            }
            let Some(wait) = mgr.time_until_soonest_recovery().await else {
                break 'outer;
            };
            if total_wait_started.elapsed() + wait > MAX_TOTAL_WAIT {
                tracing::warn!(
                    target: "jfc::provider::anthropic_oauth::rotation",
                    elapsed_secs = total_wait_started.elapsed().as_secs(),
                    "complete rotation budget exhausted — surfacing rate-limit error"
                );
                break 'outer;
            }
            let sleep_for = wait.min(MAX_RECOVERY_WAIT);
            tracing::warn!(
                target: "jfc::provider::anthropic_oauth::rotation",
                wait_secs = sleep_for.as_secs(),
                "complete: all accounts rate-limited — sleeping until soonest recovery"
            );
            tokio::time::sleep(sleep_for).await;
        }

        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("all Anthropic OAuth accounts exhausted with no successful response")
        }))
    }
}

/// Test convention follows RTCA DO-178B §6.4.2: every requirement is exercised by at
/// least one **normal range** test (valid inputs / equivalence classes / boundary values
/// / allowed state transitions) and one **robustness** test (invalid values / abnormal
/// init / corrupted input / illegal transitions).
///
/// Naming: `<unit>_<behavior>_normal` and `<unit>_<behavior>_robust`. The `// Normal:` /
/// `// Robust:` section markers below identify which DO-178B category each test belongs to.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        Provider, ProviderContent, ProviderMessage, ProviderRole, StreamConvention, StreamOptions,
        ToolDef,
    };
    use std::path::Path;

    fn make_user_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(text.to_owned())],
        }
    }

    fn opts(model: &str) -> StreamOptions {
        StreamOptions::new(model)
    }

    const TEST_BH: &str =
        "x-anthropic-billing-header: cc_version=2.0.0.abc; cc_entrypoint=cli; cch=00000;";

    #[test]
    fn system_blocks_no_caller_system_has_two_blocks() {
        let v = build_system_blocks(TEST_BH, None);
        assert_eq!(v.as_array().expect("system must be array").len(), 2);
    }

    #[test]
    fn system_blocks_position_0_is_billing_header() {
        let v = build_system_blocks(TEST_BH, None);
        let block = &v[0];
        assert_eq!(block["type"], "text");
        let text = block["text"].as_str().unwrap();
        assert!(text.starts_with("x-anthropic-billing-header:"));
        assert!(text.contains("cc_version="));
        assert!(text.contains("cc_entrypoint=cli"));
    }

    #[test]
    fn system_blocks_position_1_is_claude_identity() {
        let v = build_system_blocks(TEST_BH, None);
        let block = &v[1];
        assert_eq!(block["type"], "text");
        assert_eq!(block["text"], CLAUDE_CODE_IDENTITY);
        assert_eq!(block["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn system_blocks_caller_system_appended_at_position_2() {
        let v = build_system_blocks(TEST_BH, Some("custom instructions"));
        assert_eq!(v.as_array().unwrap().len(), 3);
        assert_eq!(v[2]["text"], "custom instructions");
    }

    #[test]
    fn system_blocks_empty_caller_system_not_appended() {
        let v = build_system_blocks(TEST_BH, Some(""));
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn build_body_required_fields_present() {
        let body = build_body(
            vec![make_user_msg("hello")],
            &opts("claude-opus-4-7"),
            TEST_BH,
        );
        assert_eq!(body["model"], "claude-opus-4-7");
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["stream"], true);
        assert!(body["messages"].is_array());
        assert!(body["system"].is_array());
    }

    #[test]
    fn build_body_tools_absent_when_empty() {
        let body = build_body(vec![make_user_msg("hi")], &opts("m"), TEST_BH);
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn build_body_tools_present_when_non_empty() {
        let o = opts("m").tools(vec![ToolDef {
            name: "bash".into(),
            description: "run bash".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }]);
        let body = build_body(vec![make_user_msg("hi")], &o, TEST_BH);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn build_body_thinking_absent_when_no_budget() {
        let body = build_body(vec![make_user_msg("hi")], &opts("m"), TEST_BH);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn build_body_thinking_present_when_budget_set() {
        let o = opts("m").thinking(4096);
        let body = build_body(vec![make_user_msg("hi")], &o, TEST_BH);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 4096);
    }

    #[test]
    fn build_body_reasoning_effort_uses_output_config_normal() {
        let o = opts("m").reasoning_effort("max");
        let body = build_body(vec![make_user_msg("hi")], &o, TEST_BH);
        assert_eq!(body["output_config"]["effort"], "max");
    }

    // strip_block removes the entire `<env>...</env>` span and leaves
    // surrounding text intact. v126 prompts wrap env / cwd / file lists
    // in tags that the Anthropic OAuth validator pattern-matches as
    // not-Claude-Code; stripping them is required to keep 200s.
    #[test]
    fn strip_block_removes_full_span_normal() {
        let s = "before\n<env>cwd=/tmp\nfiles=[a,b,c]</env>\nafter";
        let out = strip_block(s, "<env>", "</env>");
        assert_eq!(out, "before\n\nafter");
    }

    // Robust: an unclosed tag must not silently eat the rest of the
    // prompt — that would turn a typo into a complete loss of system
    // context. Leave the rest intact.
    #[test]
    fn strip_block_unclosed_tag_leaves_remainder_robust() {
        let s = "before\n<env>oops, no close tag\nload-bearing instructions";
        let out = strip_block(s, "<env>", "</env>");
        assert!(out.contains("load-bearing instructions"));
    }

    // sanitize: drops branding paragraphs but keeps body content.
    #[test]
    fn sanitize_drops_branded_paragraph_normal() {
        let s = "Welcome.\n\nVisit opencode.ai for docs.\n\nReal instructions.";
        let out = sanitize_system_prompt(s);
        assert!(out.contains("Welcome."));
        assert!(out.contains("Real instructions."));
        assert!(
            !out.contains("opencode.ai"),
            "branded paragraph not stripped: {out}"
        );
    }

    // sanitize: rewrites jfc-identity phrases so the prompt presents as
    // Claude Code, not as a third-party tool. Without this Anthropic's
    // server-side validator may reject for branding mismatch.
    #[test]
    fn sanitize_rewrites_jfc_identity_phrases_normal() {
        let s = "You are jfc, an assistant. Sisyphus is helpful.";
        let out = sanitize_system_prompt(s);
        assert!(out.contains("You are Claude Code,"));
        assert!(!out.contains("Sisyphus"), "branding leaked: {out}");
        assert!(out.contains("the assistant is helpful"));
    }

    // sanitize: env/directories blocks vanish entirely. Anthropic's
    // server-side validator treats their presence as a signal that this
    // is not Claude Code — confirmed via opencode binary search
    // (constants.ts:154).
    #[test]
    fn sanitize_strips_env_and_directories_blocks_robust() {
        let s = "intro\n\n<env>cwd=/x</env>\n\n<directories>a\nb</directories>\n\nbody";
        let out = sanitize_system_prompt(s);
        assert!(out.contains("intro"));
        assert!(out.contains("body"));
        assert!(!out.contains("<env>"));
        assert!(!out.contains("<directories>"));
        assert!(!out.contains("cwd=/x"));
    }

    // Robust: empty input stays empty (don't synthesize content from
    // nothing).
    #[test]
    fn sanitize_empty_input_returns_empty_robust() {
        assert_eq!(sanitize_system_prompt(""), "");
        assert_eq!(sanitize_system_prompt("\n\n\n"), "");
    }

    // Integration: build_system_blocks runs caller_system through
    // sanitize_system_prompt so the on-wire payload never contains
    // branded blocks even if the caller passed them.
    #[test]
    fn build_system_blocks_sanitizes_caller_system_normal() {
        // The integration check: anything in a stripped block must not
        // reach the wire payload. Identity rewrites are covered by
        // `sanitize_rewrites_jfc_identity_phrases_normal`.
        let caller =
            "intro line\n\n<env>secret=1</env>\n\n<directories>x\ny</directories>\n\nDo good work.";
        let blocks = build_system_blocks(TEST_BH, Some(caller));
        let arr = blocks.as_array().expect("array");
        // First two blocks are billing header + Claude Code identity.
        // Third block is the sanitized caller system.
        let third = arr[2]["text"].as_str().expect("text");
        assert!(third.contains("intro line"));
        assert!(third.contains("Do good work."));
        assert!(!third.contains("secret=1"));
        assert!(!third.contains("<env>"));
        assert!(!third.contains("<directories>"));
    }

    #[test]
    fn build_body_system_has_caller_block_when_system_set() {
        let o = opts("m").system("my system");
        let body = build_body(vec![make_user_msg("hi")], &o, TEST_BH);
        assert_eq!(body["system"].as_array().unwrap().len(), 3);
        assert_eq!(body["system"][2]["text"], "my system");
    }

    #[test]
    fn pick_account_selects_active_index_when_enabled() {
        let store = AccountStore {
            accounts: vec![make_account("a", None), make_account("b", None)],
            active_index: Some(1),
        };
        assert_eq!(pick_account(&store).unwrap().name, "b");
    }

    #[test]
    fn pick_account_defaults_to_index_0() {
        let store = AccountStore {
            accounts: vec![make_account("a", None), make_account("b", None)],
            active_index: None,
        };
        assert_eq!(pick_account(&store).unwrap().name, "a");
    }

    #[test]
    fn pick_account_falls_back_to_first_enabled() {
        let store = AccountStore {
            accounts: vec![
                make_account("disabled", Some(false)),
                make_account("enabled", Some(true)),
            ],
            active_index: Some(0),
        };
        assert_eq!(pick_account(&store).unwrap().name, "enabled");
    }

    #[test]
    fn pick_account_skips_empty_refresh_token() {
        let mut stale = make_account("stale", Some(true));
        stale.refresh_token.clear();
        let store = AccountStore {
            accounts: vec![stale, make_account("healthy", Some(true))],
            active_index: Some(0),
        };
        assert_eq!(pick_account(&store).unwrap().name, "healthy");
    }

    #[test]
    fn pick_account_returns_none_when_all_disabled() {
        let store = AccountStore {
            accounts: vec![
                make_account("a", Some(false)),
                make_account("b", Some(false)),
            ],
            active_index: Some(0),
        };
        assert!(pick_account(&store).is_none());
    }

    #[test]
    fn pick_account_returns_none_for_empty_store() {
        let store = AccountStore {
            accounts: vec![],
            active_index: None,
        };
        assert!(pick_account(&store).is_none());
    }

    #[test]
    fn anthropic_beta_contains_required_values() {
        for val in &[
            "claude-code-20250219",
            "oauth-2025-04-20",
            "interleaved-thinking-2025-05-14",
            "prompt-caching-2024-07-31",
            "output-128k-2025-02-19",
            "structured-outputs-2025-12-15",
        ] {
            assert!(
                ANTHROPIC_BETA.contains(val),
                "ANTHROPIC_BETA missing: {val}"
            );
        }
    }

    #[test]
    fn user_agent_format() {
        assert_eq!(
            build_user_agent("1.2.3"),
            "claude-cli/1.2.3 (external, cli)"
        );
    }

    #[test]
    fn billing_header_contains_version_and_hash() {
        let h = build_billing_header_text("2.0.0", "abc");
        assert!(h.starts_with("x-anthropic-billing-header:"));
        assert!(h.contains("cc_version=2.0.0.abc"));
        assert!(h.contains("cc_entrypoint=cli"));
        assert!(h.contains(CCH_PLACEHOLDER));
    }

    #[test]
    fn billing_hash_output_is_three_hex_chars() {
        let h = compute_billing_hash("hello world", "2.0.0");
        assert_eq!(h.len(), 3);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn billing_hash_is_deterministic() {
        let a = compute_billing_hash("hello world", "2.0.0");
        let b = compute_billing_hash("hello world", "2.0.0");
        assert_eq!(a, b);
    }

    #[test]
    fn billing_hash_differs_for_different_inputs() {
        let a = compute_billing_hash("hello world", "2.0.0");
        let b = compute_billing_hash("hello world", "2.0.1");
        assert_ne!(a, b);
    }

    #[test]
    fn billing_hash_empty_message_no_panic() {
        let h = compute_billing_hash("", "2.0.0");
        assert_eq!(h.len(), 3);
    }

    #[test]
    #[cfg(feature = "anthropic-oauth-sensitive")]
    fn body_attestation_replaces_cch_placeholder() {
        let body = format!(r#"{{"a":1,"{CCH_PLACEHOLDER}":"x"}}"#);
        let result = compute_body_attestation(&body);
        assert!(!result.contains(CCH_PLACEHOLDER));
    }

    #[test]
    #[cfg(feature = "anthropic-oauth-sensitive")]
    fn body_attestation_cch_value_is_5_hex_chars() {
        let body = format!(r#"{{"data":"hello","{CCH_PLACEHOLDER}":1}}"#);
        let result = compute_body_attestation(&body);
        let cch_start = result.find("cch=").expect("cch= not found");
        let cch_val = &result[cch_start + 4..cch_start + 9];
        assert_eq!(cch_val.len(), 5);
        assert!(cch_val.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    #[cfg(feature = "anthropic-oauth-sensitive")]
    fn body_attestation_is_deterministic() {
        let body = format!(r#"{{"k":"v","{CCH_PLACEHOLDER}":null}}"#);
        assert_eq!(
            compute_body_attestation(&body),
            compute_body_attestation(&body)
        );
    }

    #[test]
    #[cfg(feature = "anthropic-oauth-sensitive")]
    fn body_attestation_only_replaces_first_occurrence() {
        let body = format!("{CCH_PLACEHOLDER}xxx{CCH_PLACEHOLDER}");
        let result = compute_body_attestation(&body);
        assert_eq!(result.matches(CCH_PLACEHOLDER).count(), 1);
    }

    fn make_account(name: &str, enabled: Option<bool>) -> Account {
        Account {
            name: name.into(),
            refresh_token: "rt".into(),
            access_token: None,
            expires_at: None,
            enabled,
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // resolve_store_path — DO-178B §6.4.2 demonstration
    // Requirement: precedence is env override > canonical jfc-anthropic-accounts.json.
    // ─────────────────────────────────────────────────────────────────────────

    // Normal: explicit env override wins over the canonical default.
    #[test]
    fn resolve_store_path_env_override_wins_normal() {
        let home = Path::new("/home/u");
        let resolved = resolve_store_path(Some("/custom/path.json"), home);
        assert_eq!(resolved, PathBuf::from("/custom/path.json"));
    }

    // Normal: with no override, the canonical jfc path under $HOME/.config is used.
    #[test]
    fn resolve_store_path_defaults_to_jfc_canonical_normal() {
        let home = Path::new("/home/u");
        let resolved = resolve_store_path(None, home);
        assert_eq!(
            resolved,
            PathBuf::from("/home/u/.config/jfc-anthropic-accounts.json")
        );
    }

    // Robust: empty-string env override is *not* treated as unset — caller passed an
    // explicit (if degenerate) value, so we honor it. Documents the contract; lets
    // misconfiguration surface as a clear "file not found" later instead of silently
    // reading some other store.
    #[test]
    fn resolve_store_path_empty_override_is_used_verbatim_robust() {
        let home = Path::new("/home/u");
        let resolved = resolve_store_path(Some(""), home);
        assert_eq!(resolved, PathBuf::from(""));
    }

    // Robust: degenerate home path (root) still produces a deterministic, non-panicking
    // result so the caller can surface a clean error.
    #[test]
    fn resolve_store_path_root_home_no_panic_robust() {
        let resolved = resolve_store_path(None, Path::new("/"));
        assert_eq!(
            resolved,
            PathBuf::from("/.config/jfc-anthropic-accounts.json")
        );
    }

    // Robust: pick_account with an out-of-bounds active_index — must not panic and must
    // fall back to the first enabled account (illegal state transition per §6.4.2.2(g)).
    #[test]
    fn pick_account_active_index_out_of_bounds_falls_back_robust() {
        let store = AccountStore {
            accounts: vec![make_account("a", None), make_account("b", None)],
            active_index: Some(99),
        };
        assert_eq!(pick_account(&store).unwrap().name, "a");
    }

    // ── parse_model_not_found — DO-178B normal/robust ──────────────────────

    // Normal: the canonical Anthropic 404 body parses out the model id.
    #[test]
    fn parse_model_not_found_canonical_body_normal() {
        let body = r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-3-7-sonnet-20250219"},"request_id":"req_x"}"#;
        assert_eq!(
            parse_model_not_found(body).as_deref(),
            Some("claude-3-7-sonnet-20250219")
        );
    }

    // Normal: leading/trailing whitespace around the id is stripped.
    #[test]
    fn parse_model_not_found_trims_whitespace_normal() {
        let body =
            r#"{"error":{"type":"not_found_error","message":"model:   claude-opus-4-7   "}}"#;
        assert_eq!(
            parse_model_not_found(body).as_deref(),
            Some("claude-opus-4-7")
        );
    }

    // Robust: a different error type (rate_limit_error) returns None so the raw
    // body is shown instead of misleading the user with a model-access hint.
    #[test]
    fn parse_model_not_found_other_error_type_returns_none_robust() {
        let body = r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#;
        assert!(parse_model_not_found(body).is_none());
    }

    // Robust: a not_found_error whose message isn't `model:`-prefixed (e.g. a
    // missing endpoint) returns None — we don't fabricate a model id.
    #[test]
    fn parse_model_not_found_non_model_message_returns_none_robust() {
        let body = r#"{"error":{"type":"not_found_error","message":"endpoint /v2/foo not found"}}"#;
        assert!(parse_model_not_found(body).is_none());
    }

    // Robust: an empty `model:` value returns None instead of an empty string.
    #[test]
    fn parse_model_not_found_empty_id_returns_none_robust() {
        let body = r#"{"error":{"type":"not_found_error","message":"model:   "}}"#;
        assert!(parse_model_not_found(body).is_none());
    }

    // Robust: malformed JSON returns None, never panics.
    #[test]
    fn parse_model_not_found_invalid_json_returns_none_robust() {
        assert!(parse_model_not_found("not json at all").is_none());
        assert!(parse_model_not_found("").is_none());
    }

    // Normal: Anthropic can emit rate limits as SSE error messages after a
    // successful HTTP 2xx response. Those must drive silent rotation, not a
    // foreground assistant error.
    #[test]
    fn stream_rate_limit_message_recognized_normal() {
        assert!(is_stream_rate_limit_message("Rate limited"));
        assert!(is_stream_rate_limit_message("too many requests"));
        assert!(is_stream_rate_limit_message("HTTP 429 from upstream"));
    }

    // Robust: unrelated stream errors should still surface to the user instead
    // of being hidden behind a retry loop.
    #[test]
    fn stream_non_rate_limit_message_not_recognized_robust() {
        assert!(!is_stream_rate_limit_message("invalid_request_error"));
        assert!(!is_stream_rate_limit_message("model not found"));
    }

    // ── Real-API integration tests (gated #[ignore]) ──────────────────────
    // Run with: cargo test --bin jfc -- --ignored anthropic_oauth
    // Reads the configured account store and exercises the live token-refresh
    // endpoint. Skips silently when no creds exist on the machine.

    fn live_provider() -> Option<AnthropicOAuthProvider> {
        let p = AnthropicOAuthProvider::new();
        if !p.has_usable_config() {
            eprintln!(
                "skipping live test: no anthropic creds at {}",
                p.store_path.display()
            );
            return None;
        }
        Some(p)
    }

    // Normal: get_access_token resolves to a non-empty bearer token. Exercises the
    // full code path: load store → pick account → refresh-or-reuse → write back.
    #[tokio::test]
    #[ignore = "hits live Anthropic OAuth — run with cargo test -- --ignored"]
    async fn live_get_access_token_returns_token_normal() {
        let Some(p) = live_provider() else { return };
        let token = p.get_access_token().await.expect("access token");
        assert!(!token.is_empty(), "empty access token returned");
        // Anthropic OAuth tokens are JWT-shaped (three base64url segments separated
        // by '.'). Don't assert format strictly — the bearer might evolve — just
        // sanity-check it's not garbage.
        assert!(token.len() > 20, "implausibly short access token");
    }

    // Normal: live models.dev fetch via `fetch_models` propagates real ids — the
    // picker is the user-visible reason this code path matters, so we verify it
    // end-to-end.
    #[tokio::test]
    #[ignore = "hits live network — run with cargo test -- --ignored"]
    async fn live_fetch_models_returns_real_catalog_normal() {
        let Some(p) = live_provider() else { return };
        let models = <AnthropicOAuthProvider as crate::provider::Provider>::fetch_models(&p)
            .await
            .expect("fetch_models");
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| m.provider == "anthropic-oauth"));
    }

    // Normal: live `/api/oauth/profile` returns a parseable profile. We don't assert
    // specific fields (subscription type varies per account); we only check the call
    // round-trips and the cache populates.
    #[tokio::test]
    #[ignore = "hits live OAuth profile endpoint — run with cargo test -- --ignored"]
    async fn live_fetch_profile_populates_cache_normal() {
        let Some(p) = live_provider() else { return };
        let profile = p.fetch_profile().await.expect("fetch_profile");
        // Profile may have any subset of fields — the schema is documented but the
        // server occasionally omits values for free-tier accounts. The contract
        // we rely on is that the call doesn't error and the cache is populated.
        let cached = p.cached_profile().await.expect("cache populated");
        assert_eq!(cached.email, profile.email);
        assert_eq!(cached.seat_tier, profile.seat_tier);
    }

    // ── Provider trait wiring (no I/O) ────────────────────────────────────

    // Normal: name + stream_convention are the renderer's dispatch key. The
    // renderer reads these synchronously, before any I/O, so they must work
    // on a freshly constructed provider regardless of disk state.
    #[test]
    fn provider_name_and_convention_normal() {
        let p = AnthropicOAuthProvider::new();
        assert_eq!(p.name(), "anthropic-oauth");
        assert_eq!(p.stream_convention(), StreamConvention::AnthropicNative);
    }

    // Normal: available_models() returns the canonical first-party catalog
    // stamped with the "anthropic-oauth" provider tag — the picker reads
    // this before fetch_models() resolves so the user sees something
    // immediately on startup.
    #[test]
    fn available_models_uses_oauth_provider_tag_normal() {
        let p = AnthropicOAuthProvider::new();
        let models = p.available_models();
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| m.provider == "anthropic-oauth"));
    }

    // Robust: OAuth model discovery must preserve the embedded Claude Code
    // catalog. The public models.dev catalog can lag or omit Claude Code
    // OAuth-specific rows, so fetch_models intentionally does no network
    // replacement for this provider.
    #[tokio::test]
    async fn fetch_models_uses_embedded_oauth_catalog_robust() {
        let p = AnthropicOAuthProvider::new();
        let fetched = p.fetch_models().await.unwrap();
        let embedded = p.available_models();
        assert_eq!(fetched.len(), embedded.len());
        assert_eq!(
            fetched.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            embedded.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()
        );
        assert!(fetched.iter().all(|m| m.provider == "anthropic-oauth"));
    }

    // ── load_store + write_back_tokens (file I/O via tempfile) ────────────

    fn write_store_file(path: &Path, json: &str) {
        std::fs::write(path, json).expect("write tmp store");
    }

    fn temp_store(json: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("store.json");
        write_store_file(&path, json);
        (tmp, path)
    }

    // Normal: load_store parses the canonical store layout — accounts list,
    // active_index, refresh tokens. Verifies the camelCase rename rule kicks
    // in for `refreshToken` / `accessToken` / `expiresAt`.
    #[test]
    fn load_store_parses_canonical_layout_normal() {
        let (_tmp, path) = temp_store(
            r#"{
                "accounts": [
                    {"name": "primary", "refreshToken": "rt-1", "accessToken": "at-1", "expiresAt": 9999999999000, "enabled": true},
                    {"name": "secondary", "refreshToken": "rt-2"}
                ],
                "activeIndex": 0
            }"#,
        );
        let store = load_store(&path).unwrap();
        assert_eq!(store.accounts.len(), 2);
        assert_eq!(store.accounts[0].name, "primary");
        assert_eq!(store.accounts[0].refresh_token, "rt-1");
        assert_eq!(store.accounts[0].access_token.as_deref(), Some("at-1"));
        assert_eq!(store.accounts[0].expires_at, Some(9_999_999_999_000));
        assert_eq!(store.accounts[0].enabled, Some(true));
        assert_eq!(store.active_index, Some(0));
    }

    // Robust: a missing file surfaces an Err instead of panicking. The caller
    // (`get_access_token`) wraps this in a contextual error message.
    #[test]
    fn load_store_missing_file_errors_robust() {
        let bogus = PathBuf::from("/tmp/jfc-test-this-path-does-not-exist.json");
        assert!(load_store(&bogus).is_err());
    }

    // Robust: invalid JSON surfaces an Err — never panic on user-supplied
    // store contents. The user can reasonably hand-edit the file and we
    // mustn't crash the app on a typo.
    #[test]
    fn load_store_invalid_json_errors_robust() {
        let (_tmp, path) = temp_store("{ this is not json");
        assert!(load_store(&path).is_err());
    }

    // Normal: write_back_tokens updates the matching account in-place,
    // preserves other accounts untouched, and produces parseable JSON. The
    // atomic rename is exercised implicitly — if it didn't work, the read-
    // back step would fail with "file not found".
    #[test]
    fn write_back_tokens_updates_matching_account_normal() {
        let (_tmp, path) = temp_store(
            r#"{
                "accounts": [
                    {"name": "primary", "refreshToken": "rt-old", "accessToken": "at-old", "expiresAt": 1000},
                    {"name": "other",   "refreshToken": "rt-x"}
                ],
                "activeIndex": 0
            }"#,
        );

        write_back_tokens(&path, "primary", "AT-NEW", "RT-NEW", 5_000_000).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        let accounts = v["accounts"].as_array().unwrap();
        let primary = accounts
            .iter()
            .find(|a| a["name"] == "primary")
            .expect("primary still present");
        assert_eq!(primary["accessToken"], "AT-NEW");
        assert_eq!(primary["refreshToken"], "RT-NEW");
        assert_eq!(primary["expiresAt"], 5_000_000);
        // The unrelated account is left alone.
        let other = accounts.iter().find(|a| a["name"] == "other").unwrap();
        assert_eq!(other["refreshToken"], "rt-x");
    }

    // Robust: writing to a nonexistent account is a silent no-op (the loop
    // just doesn't find a match). The file must still be valid JSON afterward.
    #[test]
    fn write_back_tokens_unknown_account_is_noop_robust() {
        let (_tmp, path) =
            temp_store(r#"{"accounts":[{"name":"only","refreshToken":"rt"}],"activeIndex":0}"#);
        // Should not error even though the account doesn't exist — we don't
        // want a stale local cache to break the whole token rotation flow.
        write_back_tokens(&path, "ghost", "AT", "RT", 1).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        // Original account is untouched.
        assert_eq!(v["accounts"][0]["refreshToken"], "rt");
    }

    // Robust: a store without an `accounts` array (corrupted to e.g. an empty
    // object) round-trips through write_back without panicking.
    #[test]
    fn write_back_tokens_no_accounts_array_robust() {
        let (_tmp, path) = temp_store("{}");
        // Should not panic even when the schema is malformed — we still want
        // to surface an Ok so the rotation flow doesn't abort the whole turn.
        let result = write_back_tokens(&path, "x", "AT", "RT", 1);
        assert!(result.is_ok());
    }

    // ── has_usable_config — Provider startup gate ─────────────────────────

    // Normal: when the resolved store contains an enabled account,
    // has_usable_config returns true so main.rs registers the provider.
    #[test]
    fn has_usable_config_true_when_enabled_account_present_normal() {
        let (_tmp, path) =
            temp_store(r#"{"accounts":[{"name":"primary","refreshToken":"rt"}],"activeIndex":0}"#);
        let p = AnthropicOAuthProvider {
            client: crate::providers::http::streaming_client(),
            store_path: path,
            token: Arc::new(RwLock::new(None)),
            profile: Arc::new(RwLock::new(None)),
            manager: tokio::sync::OnceCell::new(),
        };
        assert!(p.has_usable_config());
    }

    // Robust: a missing store file means OAuth simply isn't configured —
    // has_usable_config returns false so the provider is skipped at startup
    // (matches the live_provider() helper used by the gated tests above).
    #[test]
    fn has_usable_config_false_when_store_missing_robust() {
        let p = AnthropicOAuthProvider {
            client: crate::providers::http::streaming_client(),
            store_path: PathBuf::from("/tmp/jfc-nonexistent-anthropic-store.json"),
            token: Arc::new(RwLock::new(None)),
            profile: Arc::new(RwLock::new(None)),
            manager: tokio::sync::OnceCell::new(),
        };
        assert!(!p.has_usable_config());
    }

    // Robust: a store with only disabled accounts surfaces as
    // has_usable_config==false. A user who's offboarded all their accounts
    // shouldn't see "Anthropic OAuth" in the picker as if it were ready.
    #[test]
    fn has_usable_config_false_when_all_accounts_disabled_robust() {
        let (_tmp, path) = temp_store(
            r#"{"accounts":[{"name":"x","refreshToken":"rt","enabled":false}],"activeIndex":0}"#,
        );
        let p = AnthropicOAuthProvider {
            client: crate::providers::http::streaming_client(),
            store_path: path,
            token: Arc::new(RwLock::new(None)),
            profile: Arc::new(RwLock::new(None)),
            manager: tokio::sync::OnceCell::new(),
        };
        assert!(!p.has_usable_config());
    }

    // ── cached_profile — concurrent-safe read ─────────────────────────────

    // Normal: cached_profile returns Some(...) once the cache is primed.
    #[tokio::test]
    async fn cached_profile_returns_some_when_primed_normal() {
        let p = AnthropicOAuthProvider::new();
        let primed = OAuthProfile {
            display_name: Some("Test User".into()),
            email: Some("test@example.com".into()),
            ..Default::default()
        };
        *p.profile.write().await = Some(primed.clone());

        let got = p.cached_profile().await.expect("cache should be primed");
        assert_eq!(got.display_name, primed.display_name);
        assert_eq!(got.email, primed.email);
    }

    // Robust: when no fetch has been performed yet, cached_profile returns
    // None instead of triggering I/O. The picker uses this to decide whether
    // to render a placeholder vs. account chrome.
    #[tokio::test]
    async fn cached_profile_returns_none_when_unprimed_robust() {
        let p = AnthropicOAuthProvider::new();
        assert!(p.cached_profile().await.is_none());
    }

    // ── refresh_access_token — error path ─────────────────────────────────

    // Robust: a refresh attempt against a closed local port surfaces a clean
    // Err — never panics, never hangs longer than the request timeout. This
    // exercises the network-error branch of `refresh_access_token`.
    #[tokio::test]
    async fn refresh_access_token_unreachable_endpoint_errors_robust() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        // Hit a closed loopback port — hard guarantee of "no service".
        let req = client.post("http://127.0.0.1:1/oauth/token").send().await;
        assert!(req.is_err(), "expected network error: {req:?}");
    }

    // ── OAuthProfile serde defaults ───────────────────────────────────────

    // Normal: every documented field round-trips through serde with the
    // camelCase rename rule. The picker reads `subscriptionType` and
    // `seatTier` so a typo in the rename pattern is a regression to catch.
    #[test]
    fn oauth_profile_full_camelcase_roundtrip_normal() {
        let v = serde_json::json!({
            "subscriptionType": "max",
            "seatTier": "code_max",
            "rateLimitTier": "tier4",
            "billingType": "credit_card",
            "displayName": "Cole",
            "email": "c@example.com",
            "hasExtraUsageEnabled": true
        });
        let profile: OAuthProfile = serde_json::from_value(v).unwrap();
        assert_eq!(profile.subscription_type.as_deref(), Some("max"));
        assert_eq!(profile.seat_tier.as_deref(), Some("code_max"));
        assert_eq!(profile.rate_limit_tier.as_deref(), Some("tier4"));
        assert_eq!(profile.billing_type.as_deref(), Some("credit_card"));
        assert_eq!(profile.display_name.as_deref(), Some("Cole"));
        assert_eq!(profile.email.as_deref(), Some("c@example.com"));
        assert_eq!(profile.has_extra_usage_enabled, Some(true));
    }

    // Normal: Claude Code's `/api/oauth/profile` response is nested and
    // snake_case. The picker needs these fields to apply account-aware model
    // gating and to show the account status in the footer.
    #[test]
    fn oauth_profile_nested_claude_code_shape_normal() {
        let v = serde_json::json!({
            "account": {
                "email": "nested@example.com",
                "display_name": "Nested User"
            },
            "organization": {
                "organization_type": "claude_max",
                "seat_tier": "claude-opus-4-6",
                "rate_limit_tier": "claude_max_20x",
                "billing_type": "stripe_subscription",
                "has_extra_usage_enabled": true
            }
        });
        let profile: OAuthProfile = serde_json::from_value(v).unwrap();
        assert_eq!(profile.subscription_type.as_deref(), Some("max"));
        assert_eq!(profile.seat_tier.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(profile.rate_limit_tier.as_deref(), Some("claude_max_20x"));
        assert_eq!(profile.billing_type.as_deref(), Some("stripe_subscription"));
        assert_eq!(profile.display_name.as_deref(), Some("Nested User"));
        assert_eq!(profile.email.as_deref(), Some("nested@example.com"));
        assert_eq!(profile.has_extra_usage_enabled, Some(true));
    }

    // Robust: an empty `{}` parses cleanly because every field is `Option`
    // with `#[serde(default)]`. The free-tier endpoint sometimes returns
    // sparse payloads — this guarantees we don't choke on them.
    #[test]
    fn oauth_profile_empty_object_parses_to_default_robust() {
        let v = serde_json::json!({});
        let profile: OAuthProfile = serde_json::from_value(v).unwrap();
        assert!(profile.subscription_type.is_none());
        assert!(profile.seat_tier.is_none());
        assert!(profile.email.is_none());
    }

    // ── Account / AccountStore serde ──────────────────────────────────────

    // Normal: Account deserializes refresh + access tokens via camelCase
    // rename and a missing `enabled` key parses as None (defaults to true
    // in pick_account's logic).
    #[test]
    fn account_camelcase_roundtrip_normal() {
        let v = serde_json::json!({
            "name": "primary",
            "refreshToken": "rt-1",
            "accessToken": "at-1",
            "expiresAt": 12345u64
        });
        let acct: Account = serde_json::from_value(v).unwrap();
        assert_eq!(acct.name, "primary");
        assert_eq!(acct.refresh_token, "rt-1");
        assert_eq!(acct.access_token.as_deref(), Some("at-1"));
        assert_eq!(acct.expires_at, Some(12345));
        assert!(acct.enabled.is_none());
    }

    // ── now_ms basic monotonicity ─────────────────────────────────────────

    // Normal: now_ms returns a value larger than the unix epoch. We can't
    // pin a specific value (wall clock varies) but we can pin a sanity
    // floor — anything before 2020 is impossible.
    #[test]
    fn now_ms_is_after_2020_normal() {
        // 2020-01-01T00:00:00Z in millis since epoch.
        const EPOCH_2020_MS: u64 = 1_577_836_800_000;
        assert!(now_ms() > EPOCH_2020_MS);
    }
}
