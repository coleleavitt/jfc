//! OpenWebUI account store — ports `opencode-openwebui-auth/src/storage.ts`.
//!
//! Wire-compatible with the opencode plugin's JSON on disk so users with
//! both opencode and jfc share one account file. Atomic writes via
//! tmp + rename, 0600 perms so a misconfigured umask doesn't leak the JWT.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One OpenWebUI account. Field names match opencode plugin's `OpenWebUIAccount`
/// (TypeScript camelCase, serialized as such via `rename_all`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub name: String,
    pub base_url: String,
    pub token: String,
    /// JWT `exp * 1000` — Unix millis. Optional for legacy accounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Unix millis the account was first stored. Optional for legacy entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<i64>,
    /// Unix millis the account was last refreshed. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// True → never use this account even if it's the `current`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// Per-day usage counters (rolling). Optional; matches plugin shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_usage: Option<DailyUsage>,
    /// Lifetime usage + per-model usage. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_usage: Option<TotalUsage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsage {
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
    #[serde(default)]
    pub first_seen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_model: Option<std::collections::HashMap<String, PerModelUsage>>,
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
    #[serde(default)]
    pub first_seen: String,
    #[serde(default)]
    pub last_seen: String,
}

/// On-disk store shape (opencode-compatible).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountStore {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(default)]
    pub accounts: std::collections::HashMap<String, Account>,
}

fn default_version() -> u32 {
    1
}

/// Resolve the OpenWebUI accounts store. Prefers
/// `~/.config/opencode/openwebui-accounts.json` (shared with the opencode
/// plugin), falls back to `~/.config/jfc/openwebui-accounts.json`. Override
/// with `JFC_OPENWEBUI_ACCOUNTS_PATH`.
pub fn default_store_path() -> PathBuf {
    if let Ok(p) = std::env::var("JFC_OPENWEBUI_ACCOUNTS_PATH") {
        return PathBuf::from(p);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let opencode = home.join(".config/opencode/openwebui-accounts.json");
    if opencode.exists() {
        return opencode;
    }
    home.join(".config/jfc/openwebui-accounts.json")
}

/// Load the store. Returns empty store if the file is missing/malformed —
/// matches the plugin's behavior (so a fresh install just yields no accounts).
pub fn load_store(path: &Path) -> AccountStore {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return AccountStore::default();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!(
            target: "jfc::provider::openwebui::store",
            error = %e,
            "store JSON malformed — returning empty"
        );
        AccountStore::default()
    })
}

/// Save the store atomically: write to .tmp-<pid>-<ts>, then rename.
/// 0600 perms so the JWT isn't world-readable. Mirrors `Storage.save` in
/// the opencode plugin.
pub fn save_store(path: &Path, store: &AccountStore) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("json.tmp-{pid}-{ts}"));
    let json = serde_json::to_string_pretty(store)?;
    std::fs::write(&tmp, json)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }

    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Get the currently-selected (or first enabled) account.
pub fn get_current(store: &AccountStore) -> Option<Account> {
    if let Some(ref name) = store.current {
        if let Some(a) = store.accounts.get(name) {
            if !a.disabled.unwrap_or(false) {
                return Some(a.clone());
            }
        }
    }
    store
        .accounts
        .values()
        .find(|a| !a.disabled.unwrap_or(false))
        .cloned()
}

/// Insert or update an account; mark as `current` if no current set.
pub fn upsert(path: &Path, account: Account) -> anyhow::Result<()> {
    let mut store = load_store(path);
    if store.version == 0 {
        store.version = 1;
    }
    let name = account.name.clone();
    store.accounts.insert(name.clone(), account);
    if store.current.is_none() {
        store.current = Some(name);
    }
    save_store(path, &store)
}

/// Remove an account by name. If it was `current`, pick the first remaining.
pub fn remove(path: &Path, name: &str) -> anyhow::Result<()> {
    let mut store = load_store(path);
    store.accounts.remove(name);
    if store.current.as_deref() == Some(name) {
        store.current = store.accounts.keys().next().cloned();
    }
    save_store(path, &store)
}

/// Set the current account. Returns false if the name doesn't exist.
pub fn set_current(path: &Path, name: &str) -> anyhow::Result<bool> {
    let mut store = load_store(path);
    if !store.accounts.contains_key(name) {
        return Ok(false);
    }
    store.current = Some(name.to_owned());
    save_store(path, &store)?;
    Ok(true)
}

/// List all accounts in stable name-sorted order.
pub fn list(store: &AccountStore) -> Vec<Account> {
    let mut out: Vec<Account> = store.accounts.values().cloned().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store_path() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("openwebui-accounts.json");
        (tmp, path)
    }

    #[test]
    fn upsert_creates_store_normal() {
        let (_tmp, path) = temp_store_path();
        let acct = Account {
            name: "u@h".into(),
            base_url: "https://example.com".into(),
            token: "tok".into(),
            ..Default::default()
        };
        upsert(&path, acct).unwrap();
        let store = load_store(&path);
        assert_eq!(store.version, 1);
        assert_eq!(store.current.as_deref(), Some("u@h"));
        assert!(store.accounts.contains_key("u@h"));
    }

    #[test]
    fn save_then_load_round_trip_normal() {
        let (_tmp, path) = temp_store_path();
        let mut store = AccountStore::default();
        store.accounts.insert(
            "a".into(),
            Account {
                name: "a".into(),
                base_url: "https://a".into(),
                token: "t".into(),
                expires_at: Some(123),
                ..Default::default()
            },
        );
        store.current = Some("a".into());
        save_store(&path, &store).unwrap();
        let loaded = load_store(&path);
        assert_eq!(loaded.current.as_deref(), Some("a"));
        assert_eq!(loaded.accounts.get("a").unwrap().expires_at, Some(123));
    }

    #[test]
    fn load_store_missing_returns_empty_robust() {
        let store = load_store(Path::new("/nonexistent/path/openwebui.json"));
        assert!(store.accounts.is_empty());
    }

    #[test]
    fn remove_picks_new_current_robust() {
        let (_tmp, path) = temp_store_path();
        upsert(
            &path,
            Account {
                name: "a".into(),
                base_url: "https://a".into(),
                token: "t".into(),
                ..Default::default()
            },
        )
        .unwrap();
        upsert(
            &path,
            Account {
                name: "b".into(),
                base_url: "https://b".into(),
                token: "t".into(),
                ..Default::default()
            },
        )
        .unwrap();
        set_current(&path, "a").unwrap();
        remove(&path, "a").unwrap();
        let store = load_store(&path);
        assert_eq!(store.current.as_deref(), Some("b"));
    }

    #[test]
    fn get_current_falls_back_to_first_enabled_robust() {
        let mut store = AccountStore::default();
        store.accounts.insert(
            "primary".into(),
            Account {
                name: "primary".into(),
                base_url: "https://p".into(),
                token: "t".into(),
                disabled: Some(true),
                ..Default::default()
            },
        );
        store.accounts.insert(
            "secondary".into(),
            Account {
                name: "secondary".into(),
                base_url: "https://s".into(),
                token: "t".into(),
                ..Default::default()
            },
        );
        store.current = Some("primary".into());
        let current = get_current(&store).unwrap();
        assert_eq!(current.name, "secondary");
    }
}
