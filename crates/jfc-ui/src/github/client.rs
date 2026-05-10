//! Async wrapper around the `gh` CLI for GitHub API access.
//!
//! Why shell out instead of using `octocrab` / a direct REST client?
//! - `gh` already solves auth (OAuth device flow, keyring, GH_TOKEN env,
//!   GHE host config). Re-implementing that in Rust would more than triple
//!   the surface area of this module.
//! - `gh` automatically respects `GH_HOST` for self-hosted GitHub Enterprise.
//! - Rate-limit responses come back as structured stderr text we can parse.
//!
//! All public methods are async and use `tokio::process::Command`. They
//! return [`GhError`] for non-success exits, including a dedicated
//! [`GhError::RateLimited`] variant that carries a system-reminder-shaped
//! message suitable for the model.

use std::process::Stdio;

use serde::Deserialize;
use tokio::process::Command;

/// Error from a `gh` CLI invocation.
#[derive(Debug, thiserror::Error)]
pub enum GhError {
    /// `gh` is not installed or not on PATH.
    #[error(
        "`gh` CLI not found on PATH — install via https://cli.github.com or set JFC_GH_BIN_OVERRIDE"
    )]
    NotInstalled,
    /// `gh auth login` has not been completed (or the token is expired).
    #[error("`gh` is installed but not authenticated — run `gh auth login`")]
    NotAuthenticated,
    /// The GitHub API returned 403/429 with `X-RateLimit-Remaining: 0`.
    /// `reminder` is a system-reminder-shaped string the caller can hand
    /// straight to the model.
    #[error("github rate limit hit: {reminder}")]
    RateLimited { reminder: String },
    /// `gh` exited non-zero for some other reason.
    #[error("gh failed (exit {code}): {stderr}")]
    Failed { code: i32, stderr: String },
    /// Process spawn / I/O failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Output was not valid UTF-8.
    #[error("gh stdout was not utf-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    /// JSON parse failure for `--json` queries.
    #[error("gh json parse failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// Parsed `gh pr view --json` payload.
///
/// Only the fields we actively use; the API returns much more. Adding fields
/// is a trivial change because `serde_json` ignores extras by default.
#[derive(Debug, Clone, Deserialize)]
pub struct Pr {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub state: String,
    pub url: String,
    #[serde(default)]
    pub comments: Vec<PrComment>,
    /// Review comments (line-level review feedback). `gh pr view` calls these
    /// `reviews` and nests comments under each review; we flatten on read.
    #[serde(default)]
    pub reviews: Vec<PrReview>,
    pub author: PrAuthor,
    #[serde(default, rename = "headRefName")]
    pub head_ref_name: String,
    #[serde(default, rename = "baseRefName")]
    pub base_ref_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrComment {
    pub author: PrAuthor,
    pub body: String,
    #[serde(default, rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrReview {
    pub author: PrAuthor,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub comments: Vec<PrComment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrAuthor {
    #[serde(default)]
    pub login: String,
}

/// Async wrapper for the `gh` CLI.
///
/// Cheap to construct (just stores the binary path). Methods spawn a
/// fresh subprocess each call so a slow API request doesn't block other
/// commands.
#[derive(Debug, Clone)]
pub struct GhClient {
    bin: String,
}

impl Default for GhClient {
    fn default() -> Self {
        // Honor the same override is_gh_installed() uses so tests can swap
        // in a stub script.
        let bin = std::env::var("JFC_GH_BIN_OVERRIDE").unwrap_or_else(|_| "gh".to_owned());
        Self { bin }
    }
}

impl GhClient {
    /// New client using the default `gh` resolution.
    pub fn new() -> Self {
        Self::default()
    }

    /// New client using an explicit binary path. Primarily for tests.
    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }

    /// Run `gh api <path> [args…]` and parse stdout as JSON.
    ///
    /// `path` is a relative API endpoint such as `repos/owner/repo/issues`
    /// or `rate_limit`. Extra args are forwarded verbatim — useful for
    /// `-H header:value`, `-X POST`, `-f field=value`, etc.
    pub async fn gh_api(
        &self,
        path: &str,
        extra_args: &[&str],
    ) -> Result<serde_json::Value, GhError> {
        let mut args = vec!["api", path];
        args.extend(extra_args.iter().copied());
        let raw = self.run(&args).await?;
        let v = serde_json::from_slice(&raw)?;
        Ok(v)
    }

    /// `gh pr view <num> --json …` — fetches a PR plus its review comments.
    pub async fn gh_pr_view(&self, num: u64) -> Result<Pr, GhError> {
        let num_s = num.to_string();
        let args = [
            "pr",
            "view",
            &num_s,
            "--json",
            "number,title,body,state,url,author,headRefName,baseRefName,comments,reviews",
        ];
        let raw = self.run(&args).await?;
        let pr = serde_json::from_slice::<Pr>(&raw)?;
        Ok(pr)
    }

    /// `gh pr comment <num> --body <body>` — posts a top-level PR comment.
    pub async fn gh_pr_comment(&self, num: u64, body: &str) -> Result<(), GhError> {
        let num_s = num.to_string();
        let args = ["pr", "comment", &num_s, "--body", body];
        self.run(&args).await?;
        Ok(())
    }

    /// `gh workflow list --json …` — lists Actions workflows in the repo.
    pub async fn gh_workflow_list(&self) -> Result<serde_json::Value, GhError> {
        let args = ["workflow", "list", "--json", "id,name,state,path"];
        let raw = self.run(&args).await?;
        let v = serde_json::from_slice(&raw)?;
        Ok(v)
    }

    /// `gh api rate_limit` shortcut for the assistant — emits a
    /// system-reminder string as the docstring says.
    pub async fn rate_limit_reminder(&self) -> Result<String, GhError> {
        let v = self.gh_api("rate_limit", &[]).await?;
        Ok(format_rate_limit_reminder(&v))
    }

    // -- internal -----------------------------------------------------------

    /// Spawn `gh <args>`, capture stdout/stderr, classify errors.
    async fn run(&self, args: &[&str]) -> Result<Vec<u8>, GhError> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let output = match cmd.output().await {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(GhError::NotInstalled);
            }
            Err(e) => return Err(GhError::Io(e)),
        };
        if output.status.success() {
            return Ok(output.stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        // Rate-limit detection: gh formats its rate-limit error as
        //   gh: API rate limit exceeded for ... (See documentation: ...)
        //   {"message": "API rate limit exceeded ..."}
        // We surface a system-reminder-shaped string the caller can
        // forward to the model so it knows to wait or retry.
        if is_rate_limit_stderr(&stderr) {
            return Err(GhError::RateLimited {
                reminder: rate_limit_reminder_from_stderr(&stderr),
            });
        }
        // Auth failures: `gh` prints either
        //   "gh: Not Found (HTTP 404)"  (when token can't see repo) or
        //   "To get started with GitHub CLI, please run: gh auth login"
        if stderr.contains("gh auth login") || stderr.contains("authentication required") {
            return Err(GhError::NotAuthenticated);
        }
        let code = output.status.code().unwrap_or(-1);
        Err(GhError::Failed { code, stderr })
    }
}

/// Returns true when stderr text matches the rate-limit error shape `gh`
/// emits. We check several phrases because the exact wording shifted across
/// `gh` releases.
pub fn is_rate_limit_stderr(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("api rate limit exceeded")
        || s.contains("you have exceeded a secondary rate limit")
        || (s.contains("rate limit") && s.contains("403"))
}

/// Build a system-reminder-shaped string from raw rate-limit stderr.
///
/// The format mirrors the in-app `<system-reminder>` blocks the agent
/// already knows how to consume — so handing this directly to the model
/// (e.g. as part of an autofix prompt) lets it back off correctly.
pub fn rate_limit_reminder_from_stderr(stderr: &str) -> String {
    let trimmed = stderr.trim();
    format!(
        "<system-reminder>\nGitHub API rate limit hit. Run `gh api rate_limit` to see the reset time and retry once the window opens.\nRaw error: {trimmed}\n</system-reminder>"
    )
}

/// Produce a system-reminder string from a parsed `rate_limit` API response.
///
/// The API returns `{ "rate": { "limit", "remaining", "reset", ... }, ... }`.
/// We summarize the headline rate (used for REST calls) so the model has
/// the timestamp + remaining quota in one line.
pub fn format_rate_limit_reminder(api_json: &serde_json::Value) -> String {
    let rate = api_json.get("rate");
    let remaining = rate
        .and_then(|r| r.get("remaining"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let limit = rate
        .and_then(|r| r.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let reset = rate
        .and_then(|r| r.get("reset"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    format!(
        "<system-reminder>\nGitHub REST quota: {remaining}/{limit} requests remaining. Resets at unix timestamp {reset}.\n</system-reminder>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- rate-limit detection --------------------------------------------

    #[test]
    fn is_rate_limit_stderr_detects_canonical_message_normal() {
        let s = "gh: API rate limit exceeded for 1.2.3.4. (See https://...)";
        assert!(is_rate_limit_stderr(s));
    }

    #[test]
    fn is_rate_limit_stderr_detects_secondary_normal() {
        let s = "gh: You have exceeded a secondary rate limit. Please wait...";
        assert!(is_rate_limit_stderr(s));
    }

    #[test]
    fn is_rate_limit_stderr_detects_403_with_rate_limit_normal() {
        let s = "gh: HTTP 403: rate limit exceeded for user";
        assert!(is_rate_limit_stderr(s));
    }

    #[test]
    fn is_rate_limit_stderr_ignores_other_errors_robust() {
        assert!(!is_rate_limit_stderr("gh: Not Found (HTTP 404)"));
        assert!(!is_rate_limit_stderr(""));
        assert!(!is_rate_limit_stderr("gh: HTTP 500: server error"));
    }

    #[test]
    fn rate_limit_reminder_includes_system_reminder_tags_normal() {
        let r = rate_limit_reminder_from_stderr("API rate limit exceeded for ...");
        assert!(r.contains("<system-reminder>"));
        assert!(r.contains("</system-reminder>"));
        assert!(r.contains("gh api rate_limit"));
    }

    #[test]
    fn format_rate_limit_reminder_extracts_quota_normal() {
        let v = serde_json::json!({
            "rate": { "limit": 5000, "remaining": 4321, "reset": 1700000000 }
        });
        let r = format_rate_limit_reminder(&v);
        assert!(r.contains("4321/5000"));
        assert!(r.contains("1700000000"));
    }

    #[test]
    fn format_rate_limit_reminder_handles_missing_fields_robust() {
        let v = serde_json::json!({});
        let r = format_rate_limit_reminder(&v);
        // Should not panic, should still wrap in tags.
        assert!(r.contains("<system-reminder>"));
        assert!(r.contains("0/0"));
    }

    // ---- run() error classification via stub binary ----------------------

    /// Spawning a binary that doesn't exist surfaces NotInstalled.
    #[tokio::test]
    async fn run_missing_binary_returns_not_installed_robust() {
        let client = GhClient::with_bin("/this/path/does/not/exist/gh");
        let res = client.gh_api("rate_limit", &[]).await;
        match res {
            Err(GhError::NotInstalled) => {}
            other => panic!("expected NotInstalled, got {other:?}"),
        }
    }

    /// Write a one-shot shim script to a tempdir and return its path. The
    /// tempdir handle is returned so the caller can keep it alive (drop
    /// removes the dir). Unlike `NamedTempFile`, writing through `std::fs`
    /// closes the handle so we don't hit ETXTBSY on Linux.
    #[cfg(unix)]
    fn write_shim(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("shim.sh");
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        (dir, path)
    }

    /// Pipes stderr through the rate-limit classifier when the binary exits
    /// nonzero with a rate-limit message.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_classifies_rate_limit_robust() {
        let (_dir, path) =
            write_shim("#!/bin/sh\n>&2 echo 'gh: API rate limit exceeded for 1.2.3.4'\nexit 1\n");
        let client = GhClient::with_bin(path.to_string_lossy().into_owned());
        let res = client.gh_api("rate_limit", &[]).await;
        match res {
            Err(GhError::RateLimited { reminder }) => {
                assert!(reminder.contains("<system-reminder>"));
                assert!(reminder.contains("rate limit"));
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// Auth failure path: stderr mentions `gh auth login` -> NotAuthenticated.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_classifies_not_authenticated_robust() {
        let (_dir, path) =
            write_shim("#!/bin/sh\n>&2 echo 'To get started, please run: gh auth login'\nexit 4\n");
        let client = GhClient::with_bin(path.to_string_lossy().into_owned());
        let res = client.gh_api("user", &[]).await;
        match res {
            Err(GhError::NotAuthenticated) => {}
            other => panic!("expected NotAuthenticated, got {other:?}"),
        }
    }

    /// Successful invocation: stub script that prints valid JSON to stdout.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_parses_json_normal() {
        let (_dir, path) = write_shim("#!/bin/sh\necho '{\"login\":\"octocat\",\"id\":1}'\n");
        let client = GhClient::with_bin(path.to_string_lossy().into_owned());
        let v = client.gh_api("user", &[]).await.expect("ok");
        assert_eq!(v["login"], "octocat");
        assert_eq!(v["id"], 1);
    }
}
