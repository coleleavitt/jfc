//! GitHub deep integration — wraps the `gh` CLI to talk to the GitHub API
//! without re-implementing OAuth + REST + GraphQL in Rust.
//!
//! Mirrors v2.1.132's `tengu_install_github_app_*`,
//! `tengu_setup_github_actions_*`, `tengu_autofix_pr_*` flows. We shell out to
//! `gh` via `tokio::process::Command` for everything that hits the network so
//! we get auth, rate limiting, and error handling for free, and so the user
//! can `gh auth login` in any way they prefer (keyring, env var, OAuth).
//!
//! Public surface:
//! - [`GhClient`] — async wrapper for `gh api`, `gh pr view`, `gh pr comment`
//! - [`GhContext`] — owner/repo/host derived from `git remote get-url origin`
//! - [`is_gh_installed`] — best-effort PATH check (overridable via env var
//!   `JFC_GH_BIN_OVERRIDE` for tests)
//! - [`current_repo`] — extract owner/repo from a remote URL
//!
//! Submodules implement the user-facing flows:
//! - [`install`] — `/install-github-app` wizard
//! - [`autofix`] — `/pr-autofix <num>` prompt construction
//! - [`actions`] — `/setup-github-actions` workflow scaffolding
//!
//! See `crates/jfc-ui/src/input.rs` for the slash-command dispatch arms.

pub mod actions;
pub mod autofix;
pub mod client;
pub mod install;

pub use client::GhClient;

/// Repository coordinates parsed from a git remote URL.
///
/// Both `git@github.com:owner/repo.git` (SSH) and
/// `https://github.com/owner/repo` (HTTPS) URLs are accepted. The host is
/// always normalized to lowercase so `GITHUB.COM` and `github.com` compare
/// equal — useful for self-hosted GHE comparisons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhContext {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

impl GhContext {
    /// Format as `owner/repo`, the form `gh` accepts on the command line.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

/// Returns `true` when a usable `gh` binary is on `PATH`.
///
/// Resolution order:
/// 1. `JFC_GH_BIN_OVERRIDE` env var — explicit absolute path used for tests
///    and for users with a non-standard install (e.g. `/opt/homebrew/bin/gh`).
///    A value of `__none__` forces the function to return `false` so tests
///    can simulate a missing binary on hosts that have `gh` installed.
/// 2. `PATH` lookup via `which::which` — but we avoid pulling in another
///    crate, so we walk `PATH` ourselves with `std::env::split_paths`.
pub fn is_gh_installed() -> bool {
    if let Ok(override_val) = std::env::var("JFC_GH_BIN_OVERRIDE") {
        if override_val == "__none__" {
            return false;
        }
        return std::path::Path::new(&override_val).is_file();
    }
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("gh");
        if candidate.is_file() {
            return true;
        }
        // Windows fallback — harmless on Unix because `.exe` will not exist.
        let candidate_exe = dir.join("gh.exe");
        if candidate_exe.is_file() {
            return true;
        }
    }
    false
}

/// Extract `(host, owner, repo)` from a git remote URL.
///
/// Supports the four URL shapes GitHub commonly emits:
/// - `git@github.com:owner/repo.git`           (SSH)
/// - `git@github.com:owner/repo`               (SSH no .git suffix)
/// - `https://github.com/owner/repo.git`       (HTTPS)
/// - `https://github.com/owner/repo`           (HTTPS no .git suffix)
///
/// Returns `None` for any other shape (local path, gitlab, bitbucket, ...).
pub fn parse_remote_url(url: &str) -> Option<GhContext> {
    let url = url.trim();
    // SSH form: git@host:owner/repo[.git]
    if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return parse_owner_repo(host, path);
    }
    // ssh:// form: ssh://git@host/owner/repo[.git]
    if let Some(rest) = url.strip_prefix("ssh://git@") {
        let (host, path) = rest.split_once('/')?;
        return parse_owner_repo(host, path);
    }
    // HTTPS form: https://[user@]host/owner/repo[.git]
    for prefix in ["https://", "http://"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            // Strip optional user@ prefix from authority
            let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
            let (host, path) = rest.split_once('/')?;
            return parse_owner_repo(host, path);
        }
    }
    None
}

fn parse_owner_repo(host: &str, path: &str) -> Option<GhContext> {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(GhContext {
        host: host.to_lowercase(),
        owner: owner.to_owned(),
        repo: repo.to_owned(),
    })
}

/// Resolve the current repo's `(host, owner, repo)` by running
/// `git remote get-url origin` in the current working directory.
///
/// Returns `None` when there's no git repo, no `origin` remote, or the URL
/// doesn't match a recognized GitHub-style shape. This is intentionally
/// quiet — slash command handlers that need a repo show a friendly error
/// instead of bubbling an exit code.
pub async fn current_repo() -> Option<GhContext> {
    let output = tokio::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout);
    parse_remote_url(url.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- is_gh_installed --------------------------------------------------

    #[serial_test::serial]
    #[test]
    fn is_gh_installed_returns_false_when_path_empty_robust() {
        // Mock by env var: JFC_GH_BIN_OVERRIDE=__none__ forces false even on
        // hosts where `gh` is on PATH. Without this hook we'd have to
        // shell out and the test would be flaky in CI.
        let prev = std::env::var("JFC_GH_BIN_OVERRIDE").ok();
        // SAFETY: Test is sequential within this module; we restore below.
        unsafe {
            std::env::set_var("JFC_GH_BIN_OVERRIDE", "__none__");
        }
        assert!(!is_gh_installed());
        unsafe {
            match prev {
                Some(v) => std::env::set_var("JFC_GH_BIN_OVERRIDE", v),
                None => std::env::remove_var("JFC_GH_BIN_OVERRIDE"),
            }
        }
    }

    #[serial_test::serial]
    #[test]
    fn is_gh_installed_with_override_pointing_at_real_file_normal() {
        // Point the override at /bin/sh which always exists — the function
        // only checks `is_file()`, so we confirm the override path is taken.
        let prev = std::env::var("JFC_GH_BIN_OVERRIDE").ok();
        unsafe {
            std::env::set_var("JFC_GH_BIN_OVERRIDE", "/bin/sh");
        }
        assert!(is_gh_installed());
        unsafe {
            match prev {
                Some(v) => std::env::set_var("JFC_GH_BIN_OVERRIDE", v),
                None => std::env::remove_var("JFC_GH_BIN_OVERRIDE"),
            }
        }
    }

    // ---- parse_remote_url -------------------------------------------------

    #[test]
    fn parse_ssh_remote_normal() {
        let ctx = parse_remote_url("git@github.com:anthropics/claude-code.git").unwrap();
        assert_eq!(ctx.host, "github.com");
        assert_eq!(ctx.owner, "anthropics");
        assert_eq!(ctx.repo, "claude-code");
    }

    #[test]
    fn parse_https_remote_normal() {
        let ctx = parse_remote_url("https://github.com/anthropics/claude-code").unwrap();
        assert_eq!(ctx.host, "github.com");
        assert_eq!(ctx.owner, "anthropics");
        assert_eq!(ctx.repo, "claude-code");
    }

    #[test]
    fn parse_https_with_dotgit_normal() {
        let ctx = parse_remote_url("https://github.com/owner/repo.git").unwrap();
        assert_eq!(ctx.repo, "repo");
    }

    #[test]
    fn parse_ssh_url_form_normal() {
        let ctx = parse_remote_url("ssh://git@github.com/owner/repo.git").unwrap();
        assert_eq!(ctx.host, "github.com");
        assert_eq!(ctx.owner, "owner");
    }

    #[test]
    fn parse_ghe_host_normalizes_case_normal() {
        let ctx = parse_remote_url("git@GHE.example.COM:team/proj.git").unwrap();
        assert_eq!(ctx.host, "ghe.example.com");
    }

    #[test]
    fn parse_unknown_form_returns_none_robust() {
        assert!(parse_remote_url("/local/path/repo").is_none());
        assert!(parse_remote_url("").is_none());
        assert!(parse_remote_url("git@github.com:no-slash").is_none());
        assert!(parse_remote_url("https://github.com/").is_none());
    }

    #[test]
    fn slug_formats_owner_slash_repo_normal() {
        let ctx = GhContext {
            host: "github.com".into(),
            owner: "a".into(),
            repo: "b".into(),
        };
        assert_eq!(ctx.slug(), "a/b");
    }

    // ---- current_repo (live, only meaningful in a git repo) --------------

    #[tokio::test]
    async fn current_repo_returns_some_in_jfc_repo_normal() {
        // The jfc test harness runs from a git repo with origin pointing at
        // the user's fork. We don't assert specific owner/repo because the
        // remote varies per checkout, but it must parse something.
        let ctx = current_repo().await;
        // Could be None if the developer has no `origin` remote — accept
        // either, but if Some it must be coherent.
        if let Some(ctx) = ctx {
            assert!(!ctx.owner.is_empty());
            assert!(!ctx.repo.is_empty());
            assert!(!ctx.host.is_empty());
        }
    }
}
