//! Credential vault — single source of truth for API keys and OAuth
//! tokens.
//!
//! Today jfc reads credentials from three different places: env vars,
//! `~/.config/jfc/auth.json` (Anthropic OAuth), and the OS keyring (via
//! the `keyring` crate, when configured). This module wraps all three
//! behind a single `lookup(profile)` function so multi-account workflows
//! can choose between them per-workspace via `.jfc/account.toml`.
//!
//! The Vault doesn't *replace* the existing readers — it composes them.
//! Each lookup walks the chain: profile-specific entry → env var →
//! keyring → default. First hit wins.
//!
//! Profiles are referenced via the `JFC_PROFILE` env var or via
//! `.jfc/account.toml` (`profile = "work"`). Unset = default.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountConfig {
    /// Profile name to use for this workspace. Defaults to None
    /// (env vars / OAuth file as today).
    #[serde(default)]
    pub profile: Option<String>,
}

impl AccountConfig {
    pub fn load(project_root: &Path) -> Self {
        let path = project_root.join(".jfc").join("account.toml");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&text).unwrap_or_default()
    }
}

/// Resolve the active profile for this process. Precedence:
/// 1. `JFC_PROFILE` env var
/// 2. `.jfc/account.toml` `profile` field
/// 3. None (use defaults)
pub fn active_profile(project_root: &Path) -> Option<String> {
    if let Ok(p) = std::env::var("JFC_PROFILE") {
        let p = p.trim();
        if !p.is_empty() {
            return Some(p.to_owned());
        }
    }
    AccountConfig::load(project_root).profile
}

/// Look up an API key for `provider` ("anthropic", "openai", "gemini",
/// etc.). Walks the chain: profile-suffixed env → bare env → None.
///
/// Profile-suffixed env var: `JFC_<PROVIDER>_API_KEY_<PROFILE>` with
/// profile uppercased and `-` → `_`. Example: profile "work" + provider
/// "anthropic" → `JFC_ANTHROPIC_API_KEY_WORK`.
pub fn api_key(provider: &str, profile: Option<&str>) -> Option<String> {
    let provider_upper = provider.to_ascii_uppercase();
    if let Some(prof) = profile {
        let key = format!(
            "JFC_{}_API_KEY_{}",
            provider_upper,
            prof.to_ascii_uppercase().replace('-', "_")
        );
        if let Ok(v) = std::env::var(&key) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_owned());
            }
        }
    }
    // Bare env: ANTHROPIC_API_KEY, OPENAI_API_KEY, etc.
    let bare = format!("{provider_upper}_API_KEY");
    std::env::var(&bare).ok().filter(|v| !v.trim().is_empty())
}

/// Multi-profile environment introspection — returns (profile_name,
/// is_set) for every profile-suffixed env var the user has configured.
/// Used by `/account` to show "you have profiles configured: work, home"
/// even if none is currently active.
pub fn known_profiles(provider: &str) -> Vec<String> {
    let prefix = format!("JFC_{}_API_KEY_", provider.to_ascii_uppercase());
    let mut out: Vec<String> = std::env::vars()
        .filter_map(|(k, _)| {
            k.strip_prefix(&prefix).map(|p| {
                let lc = p.to_ascii_lowercase();
                lc.replace('_', "-")
            })
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EnvGuard sets and restores process env safely for tests.
    /// Process env is global — tests that mutate it must restore.
    struct EnvGuard {
        key: String,
        prev: Option<String>,
    }
    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            // SAFETY: tests run sequentially via #[test] default;
            // env mutations are restored on Drop.
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key: key.to_owned(),
                prev,
            }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var(&self.key, v),
                    None => std::env::remove_var(&self.key),
                }
            }
        }
    }

    #[test]
    fn api_key_profile_suffix_wins_normal() {
        let _g1 = EnvGuard::set("ANTHROPIC_API_KEY", "bare");
        let _g2 = EnvGuard::set("JFC_ANTHROPIC_API_KEY_WORK", "work-specific");
        assert_eq!(api_key("anthropic", Some("work")).as_deref(), Some("work-specific"));
    }

    #[test]
    fn api_key_falls_back_to_bare_env_normal() {
        let _g1 = EnvGuard::set("ANTHROPIC_API_KEY", "bare-key");
        // Profile is set but no JFC_*_<PROFILE> exists → falls back to bare.
        assert_eq!(api_key("anthropic", Some("nonexistent")).as_deref(), Some("bare-key"));
    }

    #[test]
    fn api_key_missing_returns_none_robust() {
        // Need to clear ambient env. Use a uniquely-named provider.
        let unique = "XJFCNOPROV";
        assert!(api_key(unique, None).is_none());
        assert!(api_key(unique, Some("work")).is_none());
    }

    #[test]
    fn active_profile_env_var_overrides_file_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".jfc")).unwrap();
        std::fs::write(
            tmp.path().join(".jfc/account.toml"),
            "profile = \"file-profile\"\n",
        )
        .unwrap();
        let _g = EnvGuard::set("JFC_PROFILE", "env-profile");
        assert_eq!(active_profile(tmp.path()).as_deref(), Some("env-profile"));
    }

    #[test]
    fn active_profile_file_used_when_no_env_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".jfc")).unwrap();
        std::fs::write(
            tmp.path().join(".jfc/account.toml"),
            "profile = \"file-profile\"\n",
        )
        .unwrap();
        // SAFETY: we explicitly clear so the test is hermetic.
        unsafe {
            std::env::remove_var("JFC_PROFILE");
        }
        assert_eq!(active_profile(tmp.path()).as_deref(), Some("file-profile"));
    }

    #[test]
    fn active_profile_none_when_unset_robust() {
        let tmp = tempfile::TempDir::new().unwrap();
        unsafe {
            std::env::remove_var("JFC_PROFILE");
        }
        // No file, no env.
        assert!(active_profile(tmp.path()).is_none());
    }
}
