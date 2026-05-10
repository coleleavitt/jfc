//! Multi-account rotation manager for Anthropic OAuth.
//!
//! Port of `opencode-anthropic-auth/src/{account-manager,account-runtime-state,storage}.ts`.
//!
//! ## Responsibilities
//!
//! - Persist a JSON store of OAuth accounts compatible with opencode-anthropic-auth.
//!   Disk format is *exactly* what opencode writes, so a single store can be shared
//!   between the two tools (opencode rotates refresh tokens; jfc reads them).
//! - Track per-account runtime state in memory: rate-limit cooldowns and consecutive
//!   failure counters. Rate-limit `resetTime` may also be persisted on disk for
//!   cross-process visibility.
//! - Pick the *best available* account for a request, tier-aware (Max 20x > Max 5x >
//!   Max > Pro > default > free), preferring the active account if it's healthy enough.
//! - Atomic read-modify-write for token rotation, account add, account disable, and
//!   refresh-token clearing on `invalid_grant`.
//!
//! ## Storage Format
//!
//! ```json
//! {
//!   "accounts": [
//!     {
//!       "name": "personal",
//!       "refreshToken": "sk-ant-oat01-...",
//!       "accessToken": "...",
//!       "expiresAt": 1700000000000,
//!       "enabled": true,
//!       "rateLimitTier": "claude_max_20x",
//!       "plan": "max",
//!       ...
//!     }
//!   ],
//!   "activeIndex": 0
//! }
//! ```
//!
//! Unknown fields are preserved verbatim via `#[serde(flatten)]` so that opencode-
//! specific data (daily usage ledgers, capabilities, organization metadata, etc.)
//! round-trips through jfc unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::fs;
use tokio::sync::Mutex;

/// Maximum cooldown after a 429 — clamps malicious `retry-after` headers.
const MAX_RATE_LIMIT: Duration = Duration::from_secs(24 * 60 * 60);

/// Buffer subtracted from `expiresAt` when checking expiry — refresh slightly
/// before the upstream cutover to avoid in-flight 401s.
const TOKEN_EXPIRY_BUFFER: Duration = Duration::from_secs(5 * 60);

/// Initial cooldown for non-429 failures (network, 5xx, etc).
const FAILURE_COOLDOWN_BASE: Duration = Duration::from_secs(10);

/// Maximum cooldown for cumulative non-429 failures.
const FAILURE_COOLDOWN_MAX: Duration = Duration::from_secs(5 * 60);

/// Maximum consecutive failures before the account is considered actively bad
/// even if not rate-limited. After this many in a row it is skipped during
/// selection until success clears the counter.
const FAILURE_THRESHOLD_BAD: u32 = 5;

/// One account entry on disk. Compatible with opencode-anthropic-auth.
///
/// Unknown fields are captured into `extra` so writes round-trip without data
/// loss — opencode persists daily usage ledgers, capabilities, and other rich
/// state that jfc shouldn't strip on save.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub name: String,
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    /// Persisted by opencode for cross-process rate-limit visibility.
    /// `0` is treated as "not rate-limited"; any positive value is a unix-ms
    /// timestamp when the cooldown ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_reset_time: Option<u64>,
    /// All other fields opencode (or future jfc) may write — preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Account {
    /// Whether the OAuth access token has expired (or is within the refresh
    /// buffer window). Never-expires (`expires_at = None`) → false.
    pub fn is_token_expired(&self) -> bool {
        match self.expires_at {
            None => false,
            Some(0) => true,
            Some(ms) => {
                let now_ms = now_ms();
                let buffer = TOKEN_EXPIRY_BUFFER.as_millis() as u64;
                now_ms >= ms.saturating_sub(buffer)
            }
        }
    }

    /// `enabled` defaults to `true` when missing — only `Some(false)` disables.
    pub fn is_enabled(&self) -> bool {
        self.enabled != Some(false)
    }

    /// Persisted disk-side rate-limit clearance check. Runtime in-memory cooldown
    /// is layered on top of this in [`AccountManager::is_account_usable`].
    pub fn is_disk_rate_limit_cleared(&self) -> bool {
        match self.rate_limit_reset_time {
            None | Some(0) => true,
            Some(ms) => now_ms() >= ms,
        }
    }
}

/// On-disk wrapper holding all accounts plus the active selection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStore {
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_index: Option<usize>,
    /// Round-trip preservation for any other top-level keys opencode writes.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Per-account in-memory state. Not serialized — derived from disk + observed
/// request outcomes during the process lifetime.
#[derive(Debug, Clone, Default)]
pub struct RuntimeState {
    /// In-process cooldown end. Layered on top of the disk-persisted
    /// `rate_limit_reset_time` so we don't burn a request and immediately get
    /// rate-limited again.
    pub cooldown_until: Option<Instant>,
    /// Most recent failure timestamp (for failure-frequency heuristics).
    pub last_failure_at: Option<Instant>,
    /// How many consecutive non-success requests against this account.
    pub consecutive_failures: u32,
    /// Most recent successful use (for LRU tie-breaking).
    pub last_success_at: Option<Instant>,
}

impl RuntimeState {
    pub fn cooldown_cleared(&self) -> bool {
        match self.cooldown_until {
            None => true,
            Some(t) => Instant::now() >= t,
        }
    }
}

/// Tier ranking — higher value wins when picking among usable accounts.
///
/// Mirrors `getTierRank` in opencode's `account-manager.ts`. Strings like
/// `"claude_max_20x"`, `"claude_pro"`, etc. come from the Claude.ai
/// `internal_tier_rate_limit_tier` field on the OAuth profile endpoint.
pub fn tier_rank(tier: Option<&str>) -> u32 {
    let Some(t) = tier else {
        return 0;
    };
    let t = t.to_ascii_lowercase();
    if t.contains("raven") {
        return 100;
    }
    if t.contains("claude_max_20x") {
        return 90;
    }
    if t.contains("claude_max_5x") {
        return 80;
    }
    if t.contains("claude_max") {
        return 70;
    }
    if t.contains("claude_pro") {
        return 50;
    }
    if t.contains("claude_ai") || t.contains("default_claude") {
        return 40;
    }
    if t.contains("free") {
        return 10;
    }
    30
}

/// The rotation manager. Cheap to clone via [`Arc`]; one instance is shared
/// across the entire process so rate-limit / failure observations from any
/// concurrent request feed back into the next selection.
#[derive(Clone)]
pub struct AccountManager {
    inner: Arc<Inner>,
}

struct Inner {
    /// Path to the accounts JSON file. All disk operations route through
    /// `disk_lock` to serialize read-modify-write cycles within this process.
    store_path: PathBuf,
    disk_lock: Mutex<()>,
    /// Authoritative in-memory snapshot of disk + runtime overlays.
    state: Mutex<ManagerState>,
}

struct ManagerState {
    store: AccountStore,
    /// Mtime of the store file at last reload, in nanoseconds since epoch.
    /// Used by [`AccountManager::reload_if_changed`] to detect external writes.
    last_mtime_ns: u128,
    runtime: HashMap<String, RuntimeState>,
}

impl AccountManager {
    /// Open (and parse, if it exists) the account store at `store_path`. A
    /// missing file produces an empty manager — no-op until a login adds an
    /// account.
    pub async fn load(store_path: PathBuf) -> anyhow::Result<Self> {
        let (store, mtime_ns) = read_store(&store_path).await?;
        let mgr = Self {
            inner: Arc::new(Inner {
                store_path,
                disk_lock: Mutex::new(()),
                state: Mutex::new(ManagerState {
                    store,
                    last_mtime_ns: mtime_ns,
                    runtime: HashMap::new(),
                }),
            }),
        };
        mgr.normalize_active_index().await;
        Ok(mgr)
    }

    /// Path to the JSON file backing this manager.
    pub fn store_path(&self) -> &Path {
        &self.inner.store_path
    }

    async fn normalize_active_index(&self) {
        let mut state = self.inner.state.lock().await;
        if let Some(idx) = state.store.active_index {
            if idx >= state.store.accounts.len() {
                state.store.active_index = if state.store.accounts.is_empty() {
                    None
                } else {
                    Some(0)
                };
            }
        }
    }

    /// Re-read the store from disk if its mtime advanced (e.g., opencode
    /// rotated a refresh token). In-memory runtime state is preserved for
    /// accounts that still exist after the reload.
    pub async fn reload_if_changed(&self) -> anyhow::Result<bool> {
        let _guard = self.inner.disk_lock.lock().await;
        let current_mtime = mtime_ns(&self.inner.store_path).await?;
        let last_mtime = self.inner.state.lock().await.last_mtime_ns;
        if current_mtime <= last_mtime {
            return Ok(false);
        }
        let (store, mtime_ns) = read_store(&self.inner.store_path).await?;
        let mut state = self.inner.state.lock().await;
        // Drop runtime state for accounts that no longer exist.
        let surviving: std::collections::HashSet<String> =
            store.accounts.iter().map(|a| a.name.clone()).collect();
        state.runtime.retain(|name, _| surviving.contains(name));
        state.store = store;
        state.last_mtime_ns = mtime_ns;
        Ok(true)
    }

    /// All accounts (snapshot). Includes disabled ones — filter at call site
    /// if you only want eligible accounts.
    pub async fn list_accounts(&self) -> Vec<Account> {
        self.inner.state.lock().await.store.accounts.clone()
    }

    /// Returns `(account, runtime_state)` pairs for diagnostic display.
    pub async fn list_with_runtime(&self) -> Vec<(Account, RuntimeState)> {
        let state = self.inner.state.lock().await;
        state
            .store
            .accounts
            .iter()
            .map(|a| {
                let rt = state.runtime.get(&a.name).cloned().unwrap_or_default();
                (a.clone(), rt)
            })
            .collect()
    }

    /// Account currently marked active in the store (or first account if the
    /// stored index is out-of-bounds).
    pub async fn active_account(&self) -> Option<Account> {
        let state = self.inner.state.lock().await;
        let idx = state.store.active_index.unwrap_or(0);
        state.store.accounts.get(idx).cloned()
    }

    fn is_account_usable(account: &Account, runtime: &RuntimeState) -> bool {
        if !account.is_enabled() {
            return false;
        }
        if !account.is_disk_rate_limit_cleared() {
            return false;
        }
        if !runtime.cooldown_cleared() {
            return false;
        }
        if runtime.consecutive_failures >= FAILURE_THRESHOLD_BAD {
            return false;
        }
        // Token expiry alone doesn't disqualify — a refresh attempt happens
        // before the request leaves. Refresh failures separately mark the
        // account via `mark_failure` / `disable_account`.
        if account.is_token_expired() && account.refresh_token.is_empty() {
            return false;
        }
        true
    }

    /// Picks the best available account for an outgoing request.
    ///
    /// Selection rules (in order):
    /// 1. The currently-active account if it is usable AND has no failures
    ///    in the recent window. Stickiness reduces churn between accounts.
    /// 2. Among all usable accounts, the highest tier. Ties broken by least
    ///    recently used, then alphabetical name.
    /// 3. If no account is usable but some have refresh tokens, the one
    ///    whose disk-persisted `rate_limit_reset_time` is soonest. Caller
    ///    must decide whether to wait or surface the error.
    /// 4. `None` — all accounts are exhausted with no path to recovery.
    pub async fn pick_next(&self) -> Option<Account> {
        let state = self.inner.state.lock().await;
        let accounts = &state.store.accounts;
        if accounts.is_empty() {
            return None;
        }
        let active_idx = state.store.active_index.unwrap_or(0);

        // Tier-1: stickiness. If active is usable and clean, prefer it.
        if let Some(active) = accounts.get(active_idx) {
            let rt = state.runtime.get(&active.name).cloned().unwrap_or_default();
            if Self::is_account_usable(active, &rt) && rt.consecutive_failures == 0 {
                return Some(active.clone());
            }
        }

        // Tier-2: best-tier usable account.
        let mut usable: Vec<(&Account, RuntimeState)> = accounts
            .iter()
            .filter_map(|a| {
                let rt = state.runtime.get(&a.name).cloned().unwrap_or_default();
                Self::is_account_usable(a, &rt).then_some((a, rt))
            })
            .collect();
        if !usable.is_empty() {
            usable.sort_by(|(a1, r1), (a2, r2)| {
                let t1 = tier_rank(a1.rate_limit_tier.as_deref());
                let t2 = tier_rank(a2.rate_limit_tier.as_deref());
                t2.cmp(&t1)
                    .then_with(|| {
                        // LRU: account with older last_success wins (None = never used = win).
                        match (r1.last_success_at, r2.last_success_at) {
                            (None, None) => std::cmp::Ordering::Equal,
                            (None, Some(_)) => std::cmp::Ordering::Less,
                            (Some(_), None) => std::cmp::Ordering::Greater,
                            (Some(t1), Some(t2)) => t1.cmp(&t2),
                        }
                    })
                    .then_with(|| a1.name.cmp(&a2.name))
            });
            return Some(usable[0].0.clone());
        }

        // Tier-3: nothing usable now — pick the soonest-recovering account
        // that still has a refresh token.
        let mut waiting: Vec<&Account> = accounts
            .iter()
            .filter(|a| a.is_enabled() && !a.refresh_token.is_empty())
            .collect();
        if waiting.is_empty() {
            return None;
        }
        waiting.sort_by_key(|a| a.rate_limit_reset_time.unwrap_or(u64::MAX));
        Some(waiting[0].clone())
    }

    /// Mark the currently-active account as having succeeded. Clears the
    /// failure counter and updates the LRU timestamp.
    pub async fn mark_success(&self, name: &str) {
        let mut state = self.inner.state.lock().await;
        let rt = state.runtime.entry(name.to_owned()).or_default();
        rt.consecutive_failures = 0;
        rt.last_success_at = Some(Instant::now());
        rt.cooldown_until = None;
    }

    /// Mark the account as having received a 429. Sets a cooldown using
    /// `retry_after` if provided, else exponential-backoff based on current
    /// failure count. `retry_after_secs` of 0 produces a 60-second floor.
    pub async fn mark_rate_limited(&self, name: &str, retry_after_secs: Option<u64>) {
        let mut state = self.inner.state.lock().await;
        let rt = state.runtime.entry(name.to_owned()).or_default();
        rt.consecutive_failures = rt.consecutive_failures.saturating_add(1);
        rt.last_failure_at = Some(Instant::now());

        let dur = match retry_after_secs {
            Some(secs) if secs > 0 => Duration::from_secs(secs).min(MAX_RATE_LIMIT),
            _ => Duration::from_secs(60).min(MAX_RATE_LIMIT),
        };
        rt.cooldown_until = Some(Instant::now() + dur);
        tracing::warn!(
            target: "jfc::provider::anthropic_oauth::rotation",
            account = %name,
            cooldown_secs = dur.as_secs(),
            consecutive_failures = rt.consecutive_failures,
            "rate-limited — applied cooldown"
        );
    }

    /// Mark the account as having failed for a non-rate-limit reason.
    /// Applies an exponential-backoff cooldown so subsequent picks skip it
    /// briefly while other accounts are tried.
    pub async fn mark_failure(&self, name: &str) {
        let mut state = self.inner.state.lock().await;
        let rt = state.runtime.entry(name.to_owned()).or_default();
        rt.consecutive_failures = rt.consecutive_failures.saturating_add(1);
        rt.last_failure_at = Some(Instant::now());
        let backoff = (FAILURE_COOLDOWN_BASE * (1u32 << rt.consecutive_failures.min(5)))
            .min(FAILURE_COOLDOWN_MAX);
        rt.cooldown_until = Some(Instant::now() + backoff);
        tracing::debug!(
            target: "jfc::provider::anthropic_oauth::rotation",
            account = %name,
            cooldown_secs = backoff.as_secs(),
            consecutive_failures = rt.consecutive_failures,
            "non-429 failure — applied cooldown"
        );
    }

    /// Atomically persist new OAuth tokens for an account, then refresh the
    /// in-memory cache. Call after a successful refresh-token exchange.
    pub async fn atomic_update_tokens(
        &self,
        name: &str,
        access_token: String,
        expires_at_ms: u64,
        new_refresh_token: Option<String>,
    ) -> anyhow::Result<()> {
        self.atomic_modify(|store| {
            let Some(account) = store.accounts.iter_mut().find(|a| a.name == name) else {
                return Err(anyhow::anyhow!("atomic_update_tokens: account '{name}' not found"));
            };
            account.access_token = Some(access_token.clone());
            account.expires_at = Some(expires_at_ms);
            if let Some(rt) = new_refresh_token.clone() {
                if rt != account.refresh_token {
                    tracing::info!(
                        target: "jfc::provider::anthropic_oauth::rotation",
                        account = %name,
                        "refresh token rotated"
                    );
                    account.refresh_token = rt;
                }
            }
            // Re-enable the account if it was disabled but now has fresh tokens.
            if account.enabled == Some(false) && expires_at_ms > now_ms() {
                account.enabled = Some(true);
                account.disabled_reason = None;
                tracing::info!(
                    target: "jfc::provider::anthropic_oauth::rotation",
                    account = %name,
                    "re-enabled after successful refresh"
                );
            }
            Ok(())
        })
        .await
    }

    /// Atomically mark the account as disabled. Used when refresh permanently
    /// fails (`invalid_grant`) — we don't want any future call to retry it.
    pub async fn atomic_disable_account(&self, name: &str, reason: &str) -> anyhow::Result<()> {
        self.atomic_modify(|store| {
            if let Some(a) = store.accounts.iter_mut().find(|a| a.name == name) {
                a.enabled = Some(false);
                a.disabled_reason = Some(reason.to_owned());
            }
            Ok(())
        })
        .await?;
        tracing::warn!(
            target: "jfc::provider::anthropic_oauth::rotation",
            account = %name,
            reason = %reason,
            "account disabled"
        );
        Ok(())
    }

    /// Atomically clear the refresh token (and disable). Mirrors opencode's
    /// `tengu_oauth_refresh_token_cleared_invalid_grant` audit path — once we
    /// know a refresh token is permanently invalid we wipe it so no other
    /// process can retry with the known-bad value.
    pub async fn atomic_clear_refresh_token(&self, name: &str) -> anyhow::Result<()> {
        self.atomic_modify(|store| {
            if let Some(a) = store.accounts.iter_mut().find(|a| a.name == name) {
                a.refresh_token = String::new();
                a.access_token = None;
                a.expires_at = None;
                a.enabled = Some(false);
                a.disabled_reason = Some("invalid_grant".to_owned());
            }
            Ok(())
        })
        .await
    }

    /// Atomically persist a rate-limit reset time to disk. Used when a 429
    /// response carries an absolute reset timestamp we want all processes to
    /// honor (not just this jfc instance).
    pub async fn atomic_set_rate_limit_reset(
        &self,
        name: &str,
        reset_ms: u64,
    ) -> anyhow::Result<()> {
        self.atomic_modify(|store| {
            if let Some(a) = store.accounts.iter_mut().find(|a| a.name == name) {
                a.rate_limit_reset_time = Some(reset_ms);
            }
            Ok(())
        })
        .await
    }

    /// Update a profile-derived field (tier, plan, email). Called after a
    /// successful login or whenever the OAuth profile endpoint is queried.
    pub async fn atomic_update_profile(
        &self,
        name: &str,
        rate_limit_tier: Option<String>,
        plan: Option<String>,
        email: Option<String>,
        uuid: Option<String>,
    ) -> anyhow::Result<()> {
        self.atomic_modify(|store| {
            if let Some(a) = store.accounts.iter_mut().find(|a| a.name == name) {
                if let Some(t) = rate_limit_tier {
                    a.rate_limit_tier = Some(t);
                }
                if let Some(p) = plan {
                    a.plan = Some(p);
                }
                if let Some(e) = email {
                    a.email = Some(e);
                }
                if let Some(u) = uuid {
                    a.uuid = Some(u);
                }
            }
            Ok(())
        })
        .await
    }

    /// Atomically add a new account. Replaces any existing entry with the
    /// same name. The new account is set as `activeIndex`. Returns an error
    /// if the name is invalid or the refresh token is empty.
    pub async fn atomic_add_account(&self, mut new_account: Account) -> anyhow::Result<()> {
        validate_account_name(&new_account.name)?;
        if new_account.refresh_token.trim().is_empty() {
            anyhow::bail!("refresh_token must not be empty");
        }
        if new_account.added_at.is_none() {
            new_account.added_at = Some(now_ms());
        }
        if new_account.enabled.is_none() {
            new_account.enabled = Some(true);
        }
        let new_name = new_account.name.clone();
        self.atomic_modify(move |store| {
            store.accounts.retain(|a| a.name != new_name);
            store.accounts.push(new_account.clone());
            store.active_index = Some(store.accounts.len() - 1);
            Ok(())
        })
        .await
    }

    /// Atomically remove an account by name. Adjusts `activeIndex` if
    /// necessary so it remains in-bounds.
    pub async fn atomic_remove_account(&self, name: &str) -> anyhow::Result<bool> {
        let mut removed = false;
        self.atomic_modify(|store| {
            let Some(idx) = store.accounts.iter().position(|a| a.name == name) else {
                return Ok(());
            };
            store.accounts.remove(idx);
            removed = true;
            if let Some(active) = store.active_index {
                if store.accounts.is_empty() {
                    store.active_index = None;
                } else if active >= store.accounts.len() {
                    store.active_index = Some(store.accounts.len() - 1);
                }
            }
            Ok(())
        })
        .await?;
        Ok(removed)
    }

    /// Set the active account by name. No-op if the name is unknown.
    pub async fn atomic_set_active(&self, name: &str) -> anyhow::Result<bool> {
        let mut found = false;
        self.atomic_modify(|store| {
            if let Some(idx) = store.accounts.iter().position(|a| a.name == name) {
                store.active_index = Some(idx);
                found = true;
            }
            Ok(())
        })
        .await?;
        Ok(found)
    }

    /// Generic atomic read-modify-write under the disk lock. Re-reads the
    /// store from disk before applying the mutation so concurrent processes
    /// don't clobber each other's writes.
    async fn atomic_modify<F>(&self, mutator: F) -> anyhow::Result<()>
    where
        F: FnOnce(&mut AccountStore) -> anyhow::Result<()>,
    {
        let _guard = self.inner.disk_lock.lock().await;
        // Re-read disk so we don't clobber an opencode-side rotation.
        let (mut store, _) = read_store(&self.inner.store_path).await?;
        mutator(&mut store)?;
        write_store(&self.inner.store_path, &store).await?;
        let mtime = mtime_ns(&self.inner.store_path).await.unwrap_or(0);
        let mut state = self.inner.state.lock().await;
        // Drop runtime entries for any account that vanished after the mutation.
        let surviving: std::collections::HashSet<String> =
            store.accounts.iter().map(|a| a.name.clone()).collect();
        state.runtime.retain(|name, _| surviving.contains(name));
        state.store = store;
        state.last_mtime_ns = mtime;
        Ok(())
    }
}

// ── helpers ───────────────────────────────────────────────────────────────

/// Whitelist for account names: alnum + a few separators, max 100 chars.
/// Mirrors opencode's `VALID_ACCOUNT_NAME_REGEX`.
fn validate_account_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() || name.len() > 100 {
        anyhow::bail!("invalid account name: length must be 1..=100");
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        anyhow::bail!("invalid account name: must start with alphanumeric");
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | '+' | ' ')) {
            anyhow::bail!("invalid account name: illegal character {c:?}");
        }
    }
    Ok(())
}

/// Read and parse the account store. A non-existent file produces an empty
/// store (not an error) — first-run or zero-account scenarios are normal.
async fn read_store(path: &Path) -> anyhow::Result<(AccountStore, u128)> {
    match fs::read(path).await {
        Ok(bytes) if bytes.is_empty() => Ok((AccountStore::default(), 0)),
        Ok(bytes) => {
            let store: AccountStore = serde_json::from_slice(&bytes).map_err(|e| {
                anyhow::anyhow!("failed to parse {}: {e}", path.display())
            })?;
            let m = mtime_ns(path).await.unwrap_or(0);
            Ok((store, m))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok((AccountStore::default(), 0))
        }
        Err(e) => Err(anyhow::anyhow!("read {}: {e}", path.display())),
    }
}

/// Atomic write: serialize to a `.tmp` sibling, then `rename`. Creates parent
/// directories on demand (e.g., first-run `~/.config/opencode/`).
async fn write_store(path: &Path, store: &AccountStore) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.ok();
    }
    let body = serde_json::to_vec_pretty(store)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &body).await?;
    fs::rename(&tmp, path).await?;
    Ok(())
}

async fn mtime_ns(path: &Path) -> anyhow::Result<u128> {
    match fs::metadata(path).await {
        Ok(meta) => Ok(meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(anyhow::anyhow!("metadata {}: {e}", path.display())),
    }
}

/// Current unix-epoch milliseconds. Used everywhere `expires_at` /
/// `rate_limit_reset_time` are compared.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_account(name: &str, tier: Option<&str>) -> Account {
        Account {
            name: name.to_owned(),
            refresh_token: "rt-test".to_owned(),
            access_token: Some("at-test".to_owned()),
            expires_at: Some(now_ms() + 60_000),
            enabled: Some(true),
            rate_limit_tier: tier.map(str::to_owned),
            plan: None,
            email: None,
            uuid: None,
            added_at: Some(now_ms()),
            last_used: None,
            disabled_reason: None,
            rate_limit_reset_time: None,
            extra: Map::new(),
        }
    }

    // Normal: tier ranking matches opencode's getTierRank.
    #[test]
    fn tier_ranking_normal() {
        assert!(tier_rank(Some("raven")) > tier_rank(Some("claude_max_20x")));
        assert!(tier_rank(Some("claude_max_20x")) > tier_rank(Some("claude_max_5x")));
        assert!(tier_rank(Some("claude_max_5x")) > tier_rank(Some("claude_max")));
        assert!(tier_rank(Some("claude_max")) > tier_rank(Some("claude_pro")));
        assert!(tier_rank(Some("claude_pro")) > tier_rank(Some("free")));
        assert_eq!(tier_rank(None), 0);
        assert_eq!(tier_rank(Some("unknown_tier")), 30);
    }

    // Robust: tier matching is case-insensitive (real profiles can vary).
    #[test]
    fn tier_ranking_case_insensitive_robust() {
        assert_eq!(
            tier_rank(Some("CLAUDE_MAX_20X")),
            tier_rank(Some("claude_max_20x")),
        );
    }

    // Normal: an empty store loads cleanly and produces no candidates.
    #[tokio::test]
    async fn empty_store_picks_nothing_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        assert!(mgr.pick_next().await.is_none());
        assert!(mgr.list_accounts().await.is_empty());
    }

    // Normal: add → pick returns that account; remove → pick returns None.
    #[tokio::test]
    async fn add_pick_remove_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        mgr.atomic_add_account(mk_account("a", Some("claude_max_5x")))
            .await
            .unwrap();
        let picked = mgr.pick_next().await.unwrap();
        assert_eq!(picked.name, "a");
        assert!(mgr.atomic_remove_account("a").await.unwrap());
        assert!(mgr.pick_next().await.is_none());
    }

    // Normal: tier ranking selects Max20x over Pro when both are usable.
    #[tokio::test]
    async fn tier_ranks_drive_selection_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        mgr.atomic_add_account(mk_account("pro", Some("claude_pro")))
            .await
            .unwrap();
        mgr.atomic_add_account(mk_account("max20x", Some("claude_max_20x")))
            .await
            .unwrap();
        // Active index points to max20x (added last). Stickiness path applies.
        let picked = mgr.pick_next().await.unwrap();
        assert_eq!(picked.name, "max20x");
        // Force-pick another account by setting active to "pro" but mark pro
        // as failed — should now prefer max20x via the tier path.
        mgr.atomic_set_active("pro").await.unwrap();
        mgr.mark_failure("pro").await;
        let picked2 = mgr.pick_next().await.unwrap();
        assert_eq!(picked2.name, "max20x");
    }

    // Robust: a rate-limited account is skipped in favor of a healthy one.
    #[tokio::test]
    async fn rate_limited_account_skipped_robust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        mgr.atomic_add_account(mk_account("a", Some("claude_max_20x")))
            .await
            .unwrap();
        mgr.atomic_add_account(mk_account("b", Some("claude_pro")))
            .await
            .unwrap();
        // After both adds, active = "b". Mark "a" as rate-limited so the only
        // healthy account left is "b".
        mgr.mark_rate_limited("a", Some(60)).await;
        let picked = mgr.pick_next().await.unwrap();
        assert_eq!(picked.name, "b");
    }

    // Edge: when ALL accounts are rate-limited but have refresh tokens,
    // pick_next returns the soonest-recovering one rather than nothing.
    #[tokio::test]
    async fn all_limited_returns_soonest_edge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        let mut a = mk_account("a", Some("claude_max"));
        a.rate_limit_reset_time = Some(now_ms() + 10_000);
        let mut b = mk_account("b", Some("claude_max"));
        b.rate_limit_reset_time = Some(now_ms() + 5_000);
        mgr.atomic_add_account(a).await.unwrap();
        mgr.atomic_add_account(b).await.unwrap();
        // Both disk-limited → fall through to "soonest reset". b resets first.
        let picked = mgr.pick_next().await.unwrap();
        assert_eq!(picked.name, "b");
    }

    // Edge: a disabled account is invisible to selection even if it's the
    // only Max-tier candidate.
    #[tokio::test]
    async fn disabled_account_invisible_edge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        let mut a = mk_account("a", Some("claude_max_20x"));
        a.enabled = Some(false);
        mgr.atomic_add_account(a).await.unwrap();
        mgr.atomic_add_account(mk_account("b", Some("claude_pro")))
            .await
            .unwrap();
        let picked = mgr.pick_next().await.unwrap();
        assert_eq!(picked.name, "b");
    }

    // Robust: invalid_grant clears the refresh token on disk (security).
    #[tokio::test]
    async fn clear_refresh_token_disables_robust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        mgr.atomic_add_account(mk_account("a", None)).await.unwrap();
        mgr.atomic_clear_refresh_token("a").await.unwrap();
        let acc = mgr.list_accounts().await.into_iter().next().unwrap();
        assert!(acc.refresh_token.is_empty());
        assert_eq!(acc.enabled, Some(false));
        assert_eq!(acc.disabled_reason.as_deref(), Some("invalid_grant"));
    }

    // Robust: an unknown JSON key (e.g., opencode's `dailyUsage`) round-trips
    // through extra → write → read without loss.
    #[tokio::test]
    async fn unknown_field_round_trips_robust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        // Write directly on disk a payload with an unknown field.
        let raw = r#"{
            "accounts": [
                {
                    "name": "a",
                    "refreshToken": "rt-abc",
                    "rateLimitTier": "claude_max",
                    "dailyUsage": { "date": "2025-01-01", "inputTokens": 42 }
                }
            ],
            "activeIndex": 0
        }"#;
        tokio::fs::write(&path, raw).await.unwrap();
        let mgr = AccountManager::load(path.clone()).await.unwrap();
        // Trigger a write via mark_success-like path: update tokens.
        mgr.atomic_update_tokens("a", "new-at".into(), now_ms() + 1_000_000, None)
            .await
            .unwrap();
        // Re-read raw and assert dailyUsage survived.
        let bytes = tokio::fs::read(&path).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let daily = &v["accounts"][0]["dailyUsage"];
        assert_eq!(daily["inputTokens"].as_u64(), Some(42));
    }

    // Edge: name validation rejects illegal characters and overlong names.
    #[test]
    fn name_validation_edge() {
        assert!(validate_account_name("personal").is_ok());
        assert!(validate_account_name("user@example.com").is_ok());
        assert!(validate_account_name("a-b_c.d+e f").is_ok());
        assert!(validate_account_name("").is_err());
        assert!(validate_account_name("-leading-dash").is_err());
        assert!(validate_account_name("contains/slash").is_err());
        assert!(validate_account_name(&"x".repeat(101)).is_err());
    }

    // Robust: mark_success clears cooldown so the account picks up again.
    #[tokio::test]
    async fn mark_success_clears_cooldown_robust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        mgr.atomic_add_account(mk_account("a", None)).await.unwrap();
        mgr.mark_failure("a").await;
        mgr.mark_success("a").await;
        let runtime = mgr.list_with_runtime().await;
        let (_, rt) = &runtime[0];
        assert_eq!(rt.consecutive_failures, 0);
        assert!(rt.cooldown_until.is_none());
    }
}
