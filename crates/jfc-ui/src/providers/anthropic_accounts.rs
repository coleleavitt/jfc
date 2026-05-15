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
//! - Pick the *best available* account for a request using a load-balancing score:
//!   tier/capacity, utilization, daily usage, recent successes, failures, and
//!   current in-flight requests all feed into the choice.
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

/// Cooldown applied when a 429 is received but no `retry-after` and no
/// unified reset are available. Mirrors CC v138's `MZ6` fallback constant.
const RATE_LIMIT_DEFAULT_FALLBACK: Duration = Duration::from_secs(60);

/// Cooldown applied to an account after a single `529 / overloaded_error`.
/// Short — the issue is server-side load, usually clears in seconds. We
/// just want the rotation loop to try a different account first.
const OVERLOADED_COOLDOWN: Duration = Duration::from_secs(2);

/// Threshold at which `mark_overloaded_529` returns `true`, signalling the
/// caller to fall back to a different model. Mirrors CC v138 `e65 = 3`
/// (cli.js line 388485).
pub const OVERLOADED_FALLBACK_THRESHOLD: u32 = 3;

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
    /// Latest `anthropic-ratelimit-unified-status` seen on this account
    /// (`allowed | allowed_warning | rejected`). Persisted so a fresh jfc
    /// process can show usage warnings without first burning a request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unified_status: Option<super::unified::UnifiedStatus>,
    /// Unix-ms timestamp from `anthropic-ratelimit-unified-reset` (the
    /// representative-claim reset). Cross-process visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unified_reset_at: Option<u64>,
    /// Last-seen `representative-claim` (`seven_day_opus`, `five_hour`, …).
    /// Used by the UI to label what's limiting this account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_type: Option<super::unified::ClaimType>,
    /// Latest `anthropic-ratelimit-unified-overage-status`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overage_status: Option<super::unified::UnifiedStatus>,
    /// Unix-ms timestamp from `anthropic-ratelimit-unified-overage-reset`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overage_reset_time: Option<u64>,
    /// Latest `anthropic-ratelimit-unified-overage-disabled-reason` (verbatim).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overage_disabled_reason: Option<String>,
    /// `true` when the primary claim is rejected but overage is still
    /// servicing the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_using_overage: Option<bool>,
    /// Fraction of the 5h window consumed at the last response (in `[0, 1]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utilization_5h: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utilization_5h_reset_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utilization_7d: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utilization_7d_reset_at: Option<u64>,
    /// Unix-ms timestamp of the last successful response that touched the
    /// utilization headers. Used to gate proactive refresh of usage display
    /// (we won't burn a probe request if a real response landed recently).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_usage_refresh_at: Option<u64>,
    /// Token usage for *today* (rotates at local midnight). Compatible with
    /// opencode's `dailyUsage` schema so a shared accounts file shows the
    /// same daily counts in both tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_usage: Option<DailyUsage>,
    /// Cumulative usage since the account was added. Includes per-model
    /// breakdown with cost. Mirrors opencode's `totalUsage`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_usage: Option<TotalUsage>,
    /// All other fields opencode (or future jfc) may write — preserved verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Today's token usage for a single account. Opencode-compatible: when
/// the local-date string flips, we reset all counters to zero and bump
/// `date` to the new ISO-8601 day.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsage {
    /// ISO 8601 local date (`YYYY-MM-DD`). On record_usage, mismatched
    /// dates trigger a reset.
    pub date: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub request_count: u64,
}

/// Cumulative usage for one account, broken down per-model. `costUsd` is
/// computed at record-time using the model pricing table in `crate::cost`,
/// not derived from token counts at read-time — so even when Anthropic
/// changes prices, the historical cost figures stay accurate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TotalUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub request_count: u64,
    #[serde(default)]
    pub cost_usd: f64,
    /// ISO 8601 date of the first request ever recorded for this account.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub first_seen: String,
    /// Per-model breakdown. Keys are normalised model IDs (the same form
    /// seen in `StreamUsage.model`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub by_model: HashMap<String, PerModelUsage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerModelUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub request_count: u64,
    #[serde(default)]
    pub cost_usd: f64,
    /// ISO 8601 date of the first request that hit this model.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub first_seen: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_seen: String,
}

/// Compact projection of one account's state for UI rendering. Cached on
/// `App` and refreshed every ~10s so the ribbon doesn't have to lock the
/// manager mutex per frame.
#[derive(Debug, Clone, Default)]
pub struct AccountSnapshot {
    pub email: Option<String>,
    pub name: String,
    pub plan: Option<String>,
    pub rate_limit_tier: Option<String>,
    pub utilization_5h: Option<f64>,
    pub utilization_7d: Option<f64>,
    pub claim: Option<super::unified::ClaimType>,
    pub overage_disabled_reason: Option<String>,
    pub is_using_overage: bool,
    pub rate_limited_until_ms: Option<u64>,
    pub daily_input_tokens: u64,
    pub daily_output_tokens: u64,
    pub total_cost_usd: f64,
    pub total_request_count: u64,
}

/// One stream's worth of token usage to record. Constructed in the OAuth
/// stream wrapper from the cumulative-delta logic that's already in
/// `event_loop.rs::StreamEvent::Usage`.
#[derive(Debug, Clone, Default)]
pub struct UsageDelta {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub model: String,
    pub cost_usd: f64,
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
    /// How many consecutive `529 / overloaded_error` responses this account
    /// has served. CC v138 hard-switches to a fallback model when this hits
    /// `OVERLOADED_FALLBACK_THRESHOLD` (3); we mirror that.
    pub consecutive_529s: u32,
    /// Number of active requests currently using this account in this process.
    /// This is deliberately runtime-only; cross-process balancing still relies
    /// on persisted cooldown/usage telemetry.
    pub in_flight: u32,
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

/// Runtime claim for an account selected for an outgoing request. Dropping it
/// releases the in-flight counter even if the stream is interrupted or the
/// caller returns through an error path.
pub struct AccountRequestGuard {
    manager: AccountManager,
    account_name: String,
    released: bool,
}

impl AccountRequestGuard {
    fn new(manager: AccountManager, account_name: String) -> Self {
        Self {
            manager,
            account_name,
            released: false,
        }
    }
}

impl Drop for AccountRequestGuard {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        let manager = self.manager.clone();
        let account_name = self.account_name.clone();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async move {
                manager.release_in_flight(&account_name).await;
            });
        }
    }
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

    /// Build a compact UI snapshot for the *currently picked* account. Uses
    /// `pick_next` semantics — the account the UI is most likely to use on
    /// the next request, not necessarily the one with `activeIndex`. Returns
    /// `None` when no account is configured.
    pub async fn snapshot_for_ui(&self) -> Option<AccountSnapshot> {
        let acct = self.pick_next().await.or(self.active_account().await)?;
        Some(AccountSnapshot {
            email: acct.email.clone(),
            name: acct.name.clone(),
            plan: acct.plan.clone(),
            rate_limit_tier: acct.rate_limit_tier.clone(),
            utilization_5h: acct.utilization_5h,
            utilization_7d: acct.utilization_7d,
            claim: acct.rate_limit_type.clone(),
            overage_disabled_reason: acct.overage_disabled_reason.clone(),
            is_using_overage: acct.is_using_overage.unwrap_or(false),
            rate_limited_until_ms: acct.rate_limit_reset_time.filter(|ms| *ms > now_ms()),
            daily_input_tokens: acct
                .daily_usage
                .as_ref()
                .map(|d| d.input_tokens)
                .unwrap_or(0),
            daily_output_tokens: acct
                .daily_usage
                .as_ref()
                .map(|d| d.output_tokens)
                .unwrap_or(0),
            total_cost_usd: acct.total_usage.as_ref().map(|t| t.cost_usd).unwrap_or(0.0),
            total_request_count: acct
                .total_usage
                .as_ref()
                .map(|t| t.request_count)
                .unwrap_or(0),
        })
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

    fn utilization_pressure(account: &Account) -> f64 {
        account
            .utilization_5h
            .into_iter()
            .chain(account.utilization_7d)
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 1.0))
            .fold(0.0, f64::max)
    }

    fn daily_token_total(account: &Account) -> u64 {
        account
            .daily_usage
            .as_ref()
            .map(|u| {
                u.input_tokens
                    .saturating_add(u.output_tokens)
                    .saturating_add(u.cache_read_tokens)
                    .saturating_add(u.cache_write_tokens)
            })
            .unwrap_or(0)
    }

    fn tier_capacity(account: &Account) -> f64 {
        match tier_rank(account.rate_limit_tier.as_deref()) {
            100 => 12.0,
            90 => 10.0,
            80 => 7.5,
            70 => 6.0,
            50 => 4.5,
            40 => 3.5,
            10 => 1.5,
            0 => 2.0,
            rank => (rank as f64 / 10.0).max(2.0),
        }
    }

    fn recent_success_penalty(runtime: &RuntimeState, now: Instant) -> f64 {
        let Some(last) = runtime.last_success_at else {
            return 0.0;
        };
        let elapsed = now.saturating_duration_since(last).as_secs_f64();
        if elapsed >= 60.0 {
            0.0
        } else {
            (60.0 - elapsed) / 60.0 * 1.5
        }
    }

    fn account_score(
        account: &Account,
        runtime: &RuntimeState,
        now: Instant,
        max_daily_tokens: u64,
    ) -> f64 {
        let daily_pressure = if max_daily_tokens == 0 {
            0.0
        } else {
            Self::daily_token_total(account) as f64 / max_daily_tokens as f64
        };
        let denominator = 1.0
            + (runtime.in_flight as f64 * 1.75)
            + (Self::utilization_pressure(account) * 4.0)
            + (daily_pressure * 2.0)
            + (runtime.consecutive_failures as f64 * 1.25)
            + Self::recent_success_penalty(runtime, now);
        Self::tier_capacity(account) / denominator.max(1.0)
    }

    fn choose_account_from_state(
        state: &ManagerState,
        exclude: &std::collections::HashSet<String>,
    ) -> Option<Account> {
        let accounts = &state.store.accounts;
        if accounts.is_empty() {
            return None;
        }

        let mut usable: Vec<(&Account, RuntimeState)> = accounts
            .iter()
            .filter_map(|a| {
                let rt = state.runtime.get(&a.name).cloned().unwrap_or_default();
                (!exclude.contains(&a.name) && Self::is_account_usable(a, &rt)).then_some((a, rt))
            })
            .collect();
        if !usable.is_empty() {
            let now = Instant::now();
            let max_daily_tokens = usable
                .iter()
                .map(|(a, _)| Self::daily_token_total(a))
                .max()
                .unwrap_or(0);
            usable.sort_by(|(a1, r1), (a2, r2)| {
                let s1 = Self::account_score(a1, r1, now, max_daily_tokens);
                let s2 = Self::account_score(a2, r2, now, max_daily_tokens);
                s2.partial_cmp(&s1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| match (r1.last_success_at, r2.last_success_at) {
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, Some(_)) => std::cmp::Ordering::Less,
                        (Some(_), None) => std::cmp::Ordering::Greater,
                        (Some(t1), Some(t2)) => t1.cmp(&t2),
                    })
                    .then_with(|| a1.name.cmp(&a2.name))
            });
            return Some(usable[0].0.clone());
        }

        // Tier-3: nothing usable now — pick the soonest-recovering account
        // that still has a refresh token.
        let mut waiting: Vec<&Account> = accounts
            .iter()
            .filter(|a| !exclude.contains(&a.name) && a.is_enabled() && !a.refresh_token.is_empty())
            .collect();
        if waiting.is_empty() {
            return None;
        }
        waiting.sort_by_key(|a| a.rate_limit_reset_time.unwrap_or(u64::MAX));
        Some(waiting[0].clone())
    }

    /// Picks the best available account for an outgoing request.
    ///
    /// Selection rules (in order):
    /// 1. Among usable accounts, choose the highest load-balancing score.
    ///    Capacity/tier increases the score; high utilization, in-flight work,
    ///    recent successes, daily usage, and failures lower it.
    /// 2. If no account is usable but some have refresh tokens, the one
    ///    whose disk-persisted `rate_limit_reset_time` is soonest. Caller
    ///    must decide whether to wait or surface the error.
    /// 3. `None` — all accounts are exhausted with no path to recovery.
    pub async fn pick_next(&self) -> Option<Account> {
        let exclude = std::collections::HashSet::new();
        self.pick_next_excluding(&exclude).await
    }

    /// Same as [`Self::pick_next`], but excludes accounts already attempted
    /// within the caller's current rotation loop.
    pub async fn pick_next_excluding(
        &self,
        exclude: &std::collections::HashSet<String>,
    ) -> Option<Account> {
        let state = self.inner.state.lock().await;
        Self::choose_account_from_state(&state, exclude)
    }

    /// Select an account for a real outbound request and increment its
    /// in-flight counter atomically with the choice. The returned guard releases
    /// the counter when dropped.
    pub async fn acquire_next_excluding(
        &self,
        exclude: &std::collections::HashSet<String>,
    ) -> Option<(Account, AccountRequestGuard)> {
        let mut state = self.inner.state.lock().await;
        let account = Self::choose_account_from_state(&state, exclude)?;
        let rt = state.runtime.entry(account.name.clone()).or_default();
        rt.in_flight = rt.in_flight.saturating_add(1);
        let guard = AccountRequestGuard::new(self.clone(), account.name.clone());
        Some((account, guard))
    }

    async fn release_in_flight(&self, name: &str) {
        let mut state = self.inner.state.lock().await;
        let rt = state.runtime.entry(name.to_owned()).or_default();
        rt.in_flight = rt.in_flight.saturating_sub(1);
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

    /// Record a 429 response with full unified header context. Updates both
    /// the in-memory cooldown AND the persisted disk fields atomically so
    /// other processes (and the next jfc launch) see the same view.
    ///
    /// Cooldown source preference (mirrors CC v138 `bx_` line 317442):
    /// 1. `retry-after-ms` / `retry-after` header
    /// 2. soonest unified-`reset` timestamp
    /// 3. soonest per-claim (5h/7d/overage) reset timestamp
    /// 4. `RATE_LIMIT_DEFAULT_FALLBACK` (60s)
    pub async fn mark_rate_limited_with_info(
        &self,
        name: &str,
        info: &super::unified::RateLimitInfo,
    ) {
        let now = now_ms();
        let dur = info
            .cooldown_hint(now)
            .unwrap_or(RATE_LIMIT_DEFAULT_FALLBACK)
            .min(MAX_RATE_LIMIT);
        let cooldown_until_ms = now.saturating_add(dur.as_millis() as u64);

        // 1) update in-memory runtime state.
        {
            let mut state = self.inner.state.lock().await;
            let rt = state.runtime.entry(name.to_owned()).or_default();
            rt.consecutive_failures = rt.consecutive_failures.saturating_add(1);
            rt.last_failure_at = Some(Instant::now());
            rt.cooldown_until = Some(Instant::now() + dur);
        }

        // 2) persist to disk (best-effort — log on failure but don't bubble).
        let info = info.clone();
        let res = self
            .atomic_modify(|store| {
                let Some(account) = store.accounts.iter_mut().find(|a| a.name == name) else {
                    return Ok(());
                };
                account.rate_limit_reset_time = Some(cooldown_until_ms);
                account.unified_status = info.unified_status;
                if info.unified_reset_ms.is_some() {
                    account.unified_reset_at = info.unified_reset_ms;
                }
                if info.claim.is_some() {
                    account.rate_limit_type = info.claim.clone();
                }
                if info.overage_status.is_some() {
                    account.overage_status = info.overage_status;
                }
                if info.overage_reset_ms.is_some() {
                    account.overage_reset_time = info.overage_reset_ms;
                }
                if info.overage_disabled_reason.is_some() {
                    account.overage_disabled_reason = info.overage_disabled_reason.clone();
                }
                account.is_using_overage = Some(info.is_using_overage);
                if info.utilization_5h.is_some() {
                    account.utilization_5h = info.utilization_5h;
                    account.utilization_5h_reset_at = info.utilization_5h_reset_ms;
                }
                if info.utilization_7d.is_some() {
                    account.utilization_7d = info.utilization_7d;
                    account.utilization_7d_reset_at = info.utilization_7d_reset_ms;
                }
                account.last_usage_refresh_at = Some(now);
                Ok(())
            })
            .await;
        if let Err(e) = res {
            tracing::warn!(
                target: "jfc::provider::anthropic_oauth::rotation",
                account = %name,
                error = %e,
                "rate-limit persistence failed (continuing with in-memory state)"
            );
        }
        tracing::warn!(
            target: "jfc::provider::anthropic_oauth::rotation",
            account = %name,
            cooldown_secs = dur.as_secs(),
            claim = ?info.claim,
            unified_status = ?info.unified_status,
            "rate-limited — applied cooldown + persisted unified state"
        );
    }

    /// Record routing telemetry from a *successful* (200) response. Updates
    /// the persisted utilization snapshot so the UI can show "5h: 47% / 7d:
    /// 12%" without an extra probe request. No cooldown is set.
    pub async fn record_routing_state(&self, name: &str, info: &super::unified::RateLimitInfo) {
        // Only persist when at least one telemetry field is present — avoids
        // a write on every API-key request (which has none of these headers).
        let has_data = info.unified_status.is_some()
            || info.utilization_5h.is_some()
            || info.utilization_7d.is_some()
            || info.unified_reset_ms.is_some();
        if !has_data {
            return;
        }
        let now = now_ms();
        let info = info.clone();
        let res = self
            .atomic_modify(|store| {
                let Some(account) = store.accounts.iter_mut().find(|a| a.name == name) else {
                    return Ok(());
                };
                if info.unified_status.is_some() {
                    account.unified_status = info.unified_status;
                }
                if info.unified_reset_ms.is_some() {
                    account.unified_reset_at = info.unified_reset_ms;
                }
                if info.claim.is_some() {
                    account.rate_limit_type = info.claim.clone();
                }
                if info.overage_status.is_some() {
                    account.overage_status = info.overage_status;
                }
                if info.overage_reset_ms.is_some() {
                    account.overage_reset_time = info.overage_reset_ms;
                }
                if info.utilization_5h.is_some() {
                    account.utilization_5h = info.utilization_5h;
                    account.utilization_5h_reset_at = info.utilization_5h_reset_ms;
                }
                if info.utilization_7d.is_some() {
                    account.utilization_7d = info.utilization_7d;
                    account.utilization_7d_reset_at = info.utilization_7d_reset_ms;
                }
                account.last_usage_refresh_at = Some(now);
                // Clear stale 429 fields once status returns to allowed.
                if matches!(
                    info.unified_status,
                    Some(super::unified::UnifiedStatus::Allowed)
                        | Some(super::unified::UnifiedStatus::AllowedWarning)
                ) {
                    account.rate_limit_reset_time = None;
                }
                Ok(())
            })
            .await;
        if let Err(e) = res {
            tracing::debug!(
                target: "jfc::provider::anthropic_oauth::rotation",
                account = %name,
                error = %e,
                "routing-state persistence failed (best-effort)"
            );
        }
    }

    /// Increment the per-account `529 / overloaded_error` counter and apply
    /// a short cooldown so we don't immediately re-hit the same shard.
    /// Returns `true` once the threshold is reached so the caller can
    /// trigger a model fallback (CC v138 line 388485, `e65 = 3`).
    pub async fn mark_overloaded_529(&self, name: &str) -> bool {
        let mut state = self.inner.state.lock().await;
        let rt = state.runtime.entry(name.to_owned()).or_default();
        rt.consecutive_529s = rt.consecutive_529s.saturating_add(1);
        rt.last_failure_at = Some(Instant::now());
        // Brief cooldown — overload is server-side and usually clears in
        // seconds. Don't burn a long cooldown like we do for 429s.
        rt.cooldown_until = Some(Instant::now() + OVERLOADED_COOLDOWN);
        let crossed = rt.consecutive_529s >= OVERLOADED_FALLBACK_THRESHOLD;
        tracing::warn!(
            target: "jfc::provider::anthropic_oauth::rotation",
            account = %name,
            consecutive_529s = rt.consecutive_529s,
            threshold = OVERLOADED_FALLBACK_THRESHOLD,
            "overloaded_error — applied short cooldown"
        );
        crossed
    }

    /// Time until the soonest-recovering account becomes usable again, given
    /// the current cooldowns and disk-persisted reset timestamps. Returns
    /// `None` when at least one account is already usable (caller should not
    /// sleep) or when no account has a known recovery time (caller should
    /// surface error).
    ///
    /// Used by the rotation loop to sleep-and-retry instead of bailing when
    /// every account has been rate-limited mid-request — mirrors CC v138's
    /// "retry in Ns · attempt N/M" UX.
    pub async fn time_until_soonest_recovery(&self) -> Option<Duration> {
        let state = self.inner.state.lock().await;
        let now = Instant::now();
        let now_ms_v = now_ms();
        let mut soonest: Option<Duration> = None;
        let mut any_usable = false;
        for account in state.store.accounts.iter() {
            if !account.is_enabled() || account.refresh_token.is_empty() {
                continue;
            }
            let rt = state
                .runtime
                .get(&account.name)
                .cloned()
                .unwrap_or_default();
            // Account is currently usable — caller shouldn't sleep at all.
            if Self::is_account_usable(account, &rt) {
                any_usable = true;
                break;
            }
            // Pick the LATER of in-memory cooldown vs disk reset.
            let mem_remaining = rt
                .cooldown_until
                .and_then(|t| t.checked_duration_since(now));
            let disk_remaining = account
                .rate_limit_reset_time
                .filter(|ms| *ms > now_ms_v)
                .map(|ms| Duration::from_millis(ms - now_ms_v));
            let recovery = match (mem_remaining, disk_remaining) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            if let Some(d) = recovery {
                soonest = Some(soonest.map_or(d, |cur| cur.min(d)));
            }
        }
        if any_usable {
            return None;
        }
        soonest
    }

    /// Atomically accumulate `delta` into the account's `dailyUsage` and
    /// `totalUsage` (per-model bucket), persisting to disk. Resets the daily
    /// bucket when the local date has flipped since the last record. The
    /// cumulative cost is summed in dollars and stored as `costUsd` so the
    /// figure stays correct even if Anthropic later changes pricing.
    ///
    /// Layout is opencode-compatible: an account file shared between jfc
    /// and opencode shows the same per-model breakdown in both tools.
    pub async fn record_usage(&self, name: &str, delta: &UsageDelta) -> anyhow::Result<()> {
        let today = today_iso();
        let delta = delta.clone();
        self.atomic_modify(move |store| {
            let Some(account) = store.accounts.iter_mut().find(|a| a.name == name) else {
                return Ok(());
            };
            // Daily — reset on date change.
            let daily = account.daily_usage.get_or_insert_with(|| DailyUsage {
                date: today.clone(),
                ..DailyUsage::default()
            });
            if daily.date != today {
                *daily = DailyUsage {
                    date: today.clone(),
                    ..DailyUsage::default()
                };
            }
            daily.input_tokens = daily.input_tokens.saturating_add(delta.input_tokens);
            daily.output_tokens = daily.output_tokens.saturating_add(delta.output_tokens);
            daily.cache_read_tokens = daily
                .cache_read_tokens
                .saturating_add(delta.cache_read_tokens);
            daily.cache_write_tokens = daily
                .cache_write_tokens
                .saturating_add(delta.cache_write_tokens);
            daily.request_count = daily.request_count.saturating_add(1);

            // Total.
            let total = account.total_usage.get_or_insert_with(|| TotalUsage {
                first_seen: today.clone(),
                ..TotalUsage::default()
            });
            if total.first_seen.is_empty() {
                total.first_seen = today.clone();
            }
            total.input_tokens = total.input_tokens.saturating_add(delta.input_tokens);
            total.output_tokens = total.output_tokens.saturating_add(delta.output_tokens);
            total.cache_read_tokens = total
                .cache_read_tokens
                .saturating_add(delta.cache_read_tokens);
            total.cache_write_tokens = total
                .cache_write_tokens
                .saturating_add(delta.cache_write_tokens);
            total.request_count = total.request_count.saturating_add(1);
            total.cost_usd += delta.cost_usd;

            // Per-model.
            let key = normalize_model_key(&delta.model);
            let pm = total.by_model.entry(key).or_insert_with(|| PerModelUsage {
                first_seen: today.clone(),
                last_seen: today.clone(),
                ..PerModelUsage::default()
            });
            if pm.first_seen.is_empty() {
                pm.first_seen = today.clone();
            }
            pm.last_seen = today.clone();
            pm.input_tokens = pm.input_tokens.saturating_add(delta.input_tokens);
            pm.output_tokens = pm.output_tokens.saturating_add(delta.output_tokens);
            pm.cache_read_tokens = pm.cache_read_tokens.saturating_add(delta.cache_read_tokens);
            pm.cache_write_tokens = pm
                .cache_write_tokens
                .saturating_add(delta.cache_write_tokens);
            pm.request_count = pm.request_count.saturating_add(1);
            pm.cost_usd += delta.cost_usd;

            account.last_used = Some(now_ms());
            Ok(())
        })
        .await
    }

    /// Reset the `consecutive_529s` counter — call after a non-overloaded
    /// success on this account so a transient cluster of 529s doesn't
    /// permanently flip future requests onto the fallback model.
    pub async fn clear_overloaded_counter(&self, name: &str) {
        let mut state = self.inner.state.lock().await;
        if let Some(rt) = state.runtime.get_mut(name) {
            rt.consecutive_529s = 0;
        }
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
                return Err(anyhow::anyhow!(
                    "atomic_update_tokens: account '{name}' not found"
                ));
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
            let store: AccountStore = serde_json::from_slice(&bytes)
                .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", path.display()))?;
            let m = mtime_ns(path).await.unwrap_or(0);
            Ok((store, m))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((AccountStore::default(), 0)),
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

/// Local-timezone ISO-8601 date string (`YYYY-MM-DD`). Opencode keys its
/// `dailyUsage` and per-model `firstSeen` / `lastSeen` on the same format,
/// so a shared accounts file rolls over consistently in both tools.
pub fn today_iso() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Strip Anthropic's date suffix (`claude-opus-4-7-20250514` → `claude-opus-4-7`)
/// and lowercase, so per-model usage rolls up across snapshot bumps.
pub fn normalize_model_key(model: &str) -> String {
    let s = model.trim().to_ascii_lowercase();
    // Date suffix is always 8 digits prefixed by a dash.
    if let Some(stripped) = s.strip_suffix(|_: char| true) {
        let _ = stripped;
    }
    if s.len() > 9 {
        let tail = &s[s.len() - 9..];
        if tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            return s[..s.len() - 9].to_owned();
        }
    }
    s
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

    // Normal: a heavily-utilized high-tier account loses to a lower-tier
    // account with more headroom. This is the key proactive balancer behavior:
    // don't wait for a 429 before moving traffic.
    #[tokio::test]
    async fn utilization_pressure_drives_selection_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        let mut maxed = mk_account("maxed", Some("claude_max_20x"));
        maxed.utilization_5h = Some(0.99);
        maxed.utilization_7d = Some(0.95);
        mgr.atomic_add_account(maxed).await.unwrap();
        mgr.atomic_add_account(mk_account("pro", Some("claude_pro")))
            .await
            .unwrap();

        let picked = mgr.pick_next().await.unwrap();
        assert_eq!(picked.name, "pro");
    }

    // Normal: same-tier accounts prefer the one with lower daily usage so
    // long sessions spread cost/quota consumption instead of hammering one
    // account until cooldown.
    #[tokio::test]
    async fn daily_usage_pressure_drives_selection_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        let mut busy = mk_account("busy", Some("claude_max_5x"));
        busy.daily_usage = Some(DailyUsage {
            date: today_iso(),
            input_tokens: 900_000,
            output_tokens: 100_000,
            ..DailyUsage::default()
        });
        mgr.atomic_add_account(busy).await.unwrap();
        mgr.atomic_add_account(mk_account("idle", Some("claude_max_5x")))
            .await
            .unwrap();

        let picked = mgr.pick_next().await.unwrap();
        assert_eq!(picked.name, "idle");
    }

    // Normal: acquire_next_excluding increments in-flight state atomically
    // with selection, so concurrent requests spread across equivalent accounts.
    #[tokio::test]
    async fn in_flight_claim_drives_selection_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        mgr.atomic_add_account(mk_account("a", Some("claude_max")))
            .await
            .unwrap();
        mgr.atomic_add_account(mk_account("b", Some("claude_max")))
            .await
            .unwrap();

        let exclude = std::collections::HashSet::new();
        let (first, guard) = mgr.acquire_next_excluding(&exclude).await.unwrap();
        assert_eq!(first.name, "a");

        let second = mgr.pick_next().await.unwrap();
        assert_eq!(second.name, "b");

        drop(guard);
        tokio::task::yield_now().await;
        let runtime = mgr.list_with_runtime().await;
        assert!(
            runtime.iter().all(|(_, rt)| rt.in_flight == 0),
            "guard drop must release in-flight counters: {runtime:?}"
        );
    }

    // Robust: callers can exclude already-tried accounts within one rotation
    // round, and the picker advances to the next healthy candidate.
    #[tokio::test]
    async fn pick_next_excluding_skips_already_tried_robust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        mgr.atomic_add_account(mk_account("pro", Some("claude_pro")))
            .await
            .unwrap();
        mgr.atomic_add_account(mk_account("max20x", Some("claude_max_20x")))
            .await
            .unwrap();

        let mut exclude = std::collections::HashSet::new();
        exclude.insert("max20x".to_owned());

        let picked = mgr.pick_next_excluding(&exclude).await.unwrap();
        assert_eq!(picked.name, "pro");
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

    // Normal: mark_rate_limited_with_info persists the unified-claim type
    // and uses retry-after to set both the in-memory cooldown AND the disk
    // rate_limit_reset_time. Verifies the round-trip through the JSON file.
    #[tokio::test]
    async fn mark_rate_limited_with_info_persists_normal() {
        use super::super::unified::{ClaimType, RateLimitInfo, UnifiedStatus};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path.clone()).await.unwrap();
        mgr.atomic_add_account(mk_account("a", Some("claude_max_20x")))
            .await
            .unwrap();
        let info = RateLimitInfo {
            retry_after: Some(Duration::from_secs(45)),
            unified_status: Some(UnifiedStatus::Rejected),
            claim: Some(ClaimType::SevenDayOpus),
            utilization_5h: Some(0.99),
            utilization_7d: Some(0.42),
            ..Default::default()
        };
        mgr.mark_rate_limited_with_info("a", &info).await;

        // Re-read from disk to confirm persistence.
        let raw = tokio::fs::read(&path).await.unwrap();
        let v: Value = serde_json::from_slice(&raw).unwrap();
        let acct = &v["accounts"][0];
        assert_eq!(acct["unifiedStatus"].as_str(), Some("rejected"));
        assert_eq!(acct["rateLimitType"].as_str(), Some("seven_day_opus"));
        assert_eq!(acct["utilization5h"].as_f64(), Some(0.99));
        assert_eq!(acct["utilization7d"].as_f64(), Some(0.42));
        assert!(acct["rateLimitResetTime"].as_u64().unwrap_or(0) > now_ms());
    }

    // Edge: time_until_soonest_recovery returns None when at least one
    // account is already usable (no need to sleep) and Some when all are
    // cooling down (caller should sleep).
    #[tokio::test]
    async fn soonest_recovery_edge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        mgr.atomic_add_account(mk_account("a", None)).await.unwrap();
        mgr.atomic_add_account(mk_account("b", None)).await.unwrap();
        // Both healthy → None (caller should not sleep).
        assert!(mgr.time_until_soonest_recovery().await.is_none());

        // Cool both — soonest recovery should pick the smaller of the two.
        mgr.mark_rate_limited("a", Some(120)).await;
        mgr.mark_rate_limited("b", Some(30)).await;
        let wait = mgr.time_until_soonest_recovery().await.unwrap();
        assert!(
            wait <= Duration::from_secs(31) && wait >= Duration::from_secs(28),
            "expected ~30s, got {wait:?}"
        );
    }

    // Robust: 529 counter increments per call and trips the threshold at
    // OVERLOADED_FALLBACK_THRESHOLD (CC v138's e65=3).
    #[tokio::test]
    async fn overloaded_counter_threshold_robust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path).await.unwrap();
        mgr.atomic_add_account(mk_account("a", None)).await.unwrap();

        for _ in 0..(OVERLOADED_FALLBACK_THRESHOLD - 1) {
            assert!(!mgr.mark_overloaded_529("a").await);
        }
        // Nth call trips threshold.
        assert!(mgr.mark_overloaded_529("a").await);

        // clear_overloaded_counter resets, so subsequent overloads start fresh.
        mgr.clear_overloaded_counter("a").await;
        assert!(!mgr.mark_overloaded_529("a").await);
    }

    // Robust: record_routing_state on a 200 with utilization headers writes
    // the snapshot to disk without setting any cooldown.
    #[tokio::test]
    async fn record_routing_state_no_cooldown_robust() {
        use super::super::unified::{RateLimitInfo, UnifiedStatus};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path.clone()).await.unwrap();
        mgr.atomic_add_account(mk_account("a", None)).await.unwrap();
        let info = RateLimitInfo {
            unified_status: Some(UnifiedStatus::AllowedWarning),
            utilization_5h: Some(0.75),
            utilization_7d: Some(0.20),
            ..Default::default()
        };
        mgr.record_routing_state("a", &info).await;
        let raw = tokio::fs::read(&path).await.unwrap();
        let v: Value = serde_json::from_slice(&raw).unwrap();
        let acct = &v["accounts"][0];
        assert_eq!(acct["unifiedStatus"].as_str(), Some("allowed_warning"));
        assert_eq!(acct["utilization5h"].as_f64(), Some(0.75));
        // No cooldown.
        let runtime = mgr.list_with_runtime().await;
        assert!(runtime[0].1.cooldown_until.is_none());
    }

    // Robust: record_routing_state with no telemetry headers is a no-op
    // (avoids writing to disk on every API-key request).
    #[tokio::test]
    async fn record_routing_state_noop_when_empty_robust() {
        use super::super::unified::RateLimitInfo;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path.clone()).await.unwrap();
        mgr.atomic_add_account(mk_account("a", None)).await.unwrap();
        let mtime_before = mtime_ns(&path).await.unwrap();
        // Sleep a tick so a write would change mtime if it happened.
        tokio::time::sleep(Duration::from_millis(15)).await;
        mgr.record_routing_state("a", &RateLimitInfo::default())
            .await;
        let mtime_after = mtime_ns(&path).await.unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "record_routing_state must not write when nothing to persist"
        );
    }

    // Normal: model-key normalization strips the trailing date suffix so
    // `claude-opus-4-7-20250514` and `claude-opus-4-7` roll into one bucket.
    #[test]
    fn normalize_model_key_strips_date_normal() {
        assert_eq!(
            normalize_model_key("claude-opus-4-7-20250514"),
            "claude-opus-4-7"
        );
        assert_eq!(normalize_model_key("claude-opus-4-7"), "claude-opus-4-7");
        assert_eq!(normalize_model_key("CLAUDE-OPUS-4-7"), "claude-opus-4-7");
        // Edge: 8 digits not preceded by a dash → not a date, leave alone.
        assert_eq!(normalize_model_key("model12345678"), "model12345678");
    }

    // Normal: record_usage accumulates daily/total/per-model counters and
    // computes cost in the byModel bucket. Verifies the JSON layout is
    // opencode-compatible (camelCase, costUsd, firstSeen, lastSeen).
    #[tokio::test]
    async fn record_usage_round_trips_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path.clone()).await.unwrap();
        mgr.atomic_add_account(mk_account("a", None)).await.unwrap();

        let delta = UsageDelta {
            input_tokens: 100,
            output_tokens: 200,
            cache_read_tokens: 50,
            cache_write_tokens: 25,
            model: "claude-opus-4-7".to_owned(),
            cost_usd: 1.25,
        };
        mgr.record_usage("a", &delta).await.unwrap();
        mgr.record_usage("a", &delta).await.unwrap();

        let raw = tokio::fs::read(&path).await.unwrap();
        let v: Value = serde_json::from_slice(&raw).unwrap();
        let acct = &v["accounts"][0];

        assert_eq!(acct["dailyUsage"]["inputTokens"].as_u64(), Some(200));
        assert_eq!(acct["dailyUsage"]["requestCount"].as_u64(), Some(2));
        assert_eq!(acct["totalUsage"]["outputTokens"].as_u64(), Some(400));
        assert_eq!(acct["totalUsage"]["requestCount"].as_u64(), Some(2));
        assert!((acct["totalUsage"]["costUsd"].as_f64().unwrap() - 2.50).abs() < 1e-9);
        let by_model = &acct["totalUsage"]["byModel"]["claude-opus-4-7"];
        assert_eq!(by_model["inputTokens"].as_u64(), Some(200));
        assert_eq!(by_model["requestCount"].as_u64(), Some(2));
        assert!(by_model["firstSeen"].as_str().unwrap().len() == 10);
    }

    // Edge: record_usage on a date change resets the daily bucket to zero
    // before adding the new delta. We simulate by hand-poking the date.
    #[tokio::test]
    async fn record_usage_resets_daily_on_date_change_edge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mgr = AccountManager::load(path.clone()).await.unwrap();
        mgr.atomic_add_account(mk_account("a", None)).await.unwrap();
        let delta = UsageDelta {
            input_tokens: 999,
            output_tokens: 999,
            model: "claude-opus-4-7".to_owned(),
            cost_usd: 5.0,
            ..Default::default()
        };
        mgr.record_usage("a", &delta).await.unwrap();

        // Simulate yesterday's bucket by writing back stale date.
        let raw = tokio::fs::read(&path).await.unwrap();
        let mut v: Value = serde_json::from_slice(&raw).unwrap();
        v["accounts"][0]["dailyUsage"]["date"] = Value::String("1999-01-01".into());
        tokio::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap())
            .await
            .unwrap();
        let mgr2 = AccountManager::load(path.clone()).await.unwrap();

        let small = UsageDelta {
            input_tokens: 1,
            output_tokens: 1,
            model: "claude-opus-4-7".to_owned(),
            cost_usd: 0.1,
            ..Default::default()
        };
        mgr2.record_usage("a", &small).await.unwrap();
        let raw = tokio::fs::read(&path).await.unwrap();
        let v: Value = serde_json::from_slice(&raw).unwrap();
        // Daily should be the new tiny delta only — yesterday's 999 is gone.
        assert_eq!(
            v["accounts"][0]["dailyUsage"]["inputTokens"].as_u64(),
            Some(1)
        );
        // Total preserves yesterday + today.
        assert_eq!(
            v["accounts"][0]["totalUsage"]["inputTokens"].as_u64(),
            Some(1000)
        );
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
