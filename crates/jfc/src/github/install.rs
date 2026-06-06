//! `/install-github-app` interactive wizard.
//!
//! Mirrors v2.1.132's `tengu_install_github_app_*` flow. The user runs
//! `/install-github-app`, we open the browser at the GitHub App install URL
//! scoped to the current repo's owner, then poll `gh api` for the app
//! installation to confirm the user finished the OAuth dance.
//!
//! This module is intentionally *display-only* — the wizard is driven by the
//! slash-command handler in `input.rs`, which owns the `ChatMessage` echo
//! and toast state. Functions here build the URLs, verify install state, and
//! emit the markdown blob the dispatcher pushes into the transcript.
//!
//! ## Auth gap
//!
//! Polling for installation requires `gh auth` because GitHub's
//! `/repos/{owner}/{repo}/installation` endpoint is authed. If the user
//! hasn't logged in to `gh`, we still emit the install URL — they can
//! click through and authorize the app, then run `gh auth login` later.

use super::GhContext;
use super::client::{GhClient, GhError};

/// The Anthropic GitHub App slug. Hard-coded because that's what v2.1.132
/// installs — flip this constant for forks of jfc that ship their own app.
pub const APP_SLUG: &str = "claude";

/// Build the GitHub install URL for the Anthropic Claude app, scoped to the
/// current repo's owner. The browser flow lets the user pick which repos
/// to grant access to.
pub fn install_url(ctx: &GhContext) -> String {
    // Format mirrors GitHub's "Install App for organization" deep link.
    // We prefix the host so GHE installs land on the right server.
    format!(
        "https://{host}/apps/{slug}/installations/new/permissions?target_id={owner}",
        host = ctx.host,
        slug = APP_SLUG,
        owner = urlencoding(&ctx.owner),
    )
}

/// Quick percent-encode for URL components — only handles the chars we
/// actually expect in owners/repos (alnum + `-`, `_`, `.`). Everything else
/// gets `%XX`-encoded. Avoids pulling in the `url` crate.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Open a URL in the user's default browser. Best-effort — failure is
/// surfaced to the caller as a string so the slash-command dispatcher can
/// fall back to printing the URL.
pub async fn open_browser(url: &str) -> Result<(), String> {
    // Pick the right opener for the platform. We deliberately don't pull
    // in the `open` crate — these three commands cover Linux/macOS/WSL.
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "cmd"
    } else {
        "xdg-open"
    };
    let mut command = tokio::process::Command::new(cmd);
    if cfg!(target_os = "windows") {
        command.args(["/C", "start", url]);
    } else {
        command.arg(url);
    }
    match command.spawn() {
        Ok(mut child) => {
            // Don't wait for the browser to close — fire-and-forget. We
            // still drop the handle to avoid leaking a zombie.
            let _ = child.try_wait();
            Ok(())
        }
        Err(e) => Err(format!("failed to launch `{cmd}`: {e}")),
    }
}

/// Returns Some(installation) when the Claude GitHub App is installed on
/// `ctx`'s repo. The "installation" object is whatever `gh api` returned —
/// we re-emit it verbatim so the caller can show the install id, etc.
pub async fn check_installed(
    client: &GhClient,
    ctx: &GhContext,
) -> Result<Option<serde_json::Value>, GhError> {
    let path = format!("repos/{}/{}/installation", ctx.owner, ctx.repo);
    match client.gh_api(&path, &[]).await {
        Ok(v) => Ok(Some(v)),
        Err(GhError::Failed { code: _, stderr }) if stderr.to_lowercase().contains("not found") => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Markdown text rendered into the chat transcript when the user runs
/// `/install-github-app`. Parameterized over the URL because `install_url`
/// composes the host + owner.
pub fn install_message(ctx: &GhContext, url: &str) -> String {
    format!(
        "**Installing Claude GitHub App** for `{slug}`\n\n\
         1. Your browser should have opened to:\n   <{url}>\n\
         2. Pick which repositories to grant access to (you can scope to just `{repo}`).\n\
         3. After authorizing, return here and run `/install-github-app` again to verify.\n\n\
         If the browser didn't open, copy the URL above. \
         If `gh` is not authenticated yet, also run `gh auth login` so jfc can verify the install.",
        slug = ctx.slug(),
        repo = ctx.repo,
    )
}

/// Markdown text shown when verification finds the app is already installed.
pub fn already_installed_message(ctx: &GhContext, install_id: Option<u64>) -> String {
    let id = install_id
        .map(|n| format!(" (id `{n}`)"))
        .unwrap_or_default();
    format!(
        "Claude GitHub App is **already installed** on `{slug}`{id}. \
         You can use `/pr <num>`, `/pr-autofix <num>`, and `/setup-github-actions` against this repo.",
        slug = ctx.slug(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_url_uses_host_normal() {
        let ctx = GhContext {
            host: "github.com".into(),
            owner: "anthropics".into(),
            repo: "claude-code".into(),
        };
        let url = install_url(&ctx);
        assert!(url.starts_with("https://github.com/apps/claude/"));
        assert!(url.contains("target_id=anthropics"));
    }

    #[test]
    fn install_url_works_for_ghe_normal() {
        let ctx = GhContext {
            host: "ghe.example.com".into(),
            owner: "team".into(),
            repo: "proj".into(),
        };
        let url = install_url(&ctx);
        assert!(url.starts_with("https://ghe.example.com/apps/claude/"));
    }

    #[test]
    fn urlencoding_passes_safe_chars_normal() {
        assert_eq!(urlencoding("plain"), "plain");
        assert_eq!(urlencoding("dot.s_and-dashes"), "dot.s_and-dashes");
    }

    #[test]
    fn urlencoding_encodes_specials_normal() {
        assert_eq!(urlencoding("a/b"), "a%2Fb");
        assert_eq!(urlencoding("hello world"), "hello%20world");
    }

    #[test]
    fn install_message_mentions_repo_normal() {
        let ctx = GhContext {
            host: "github.com".into(),
            owner: "a".into(),
            repo: "b".into(),
        };
        let msg = install_message(&ctx, "https://example/url");
        assert!(msg.contains("a/b"));
        assert!(msg.contains("https://example/url"));
        assert!(msg.contains("gh auth login"));
    }

    #[test]
    fn already_installed_message_shows_id_when_some_normal() {
        let ctx = GhContext {
            host: "github.com".into(),
            owner: "a".into(),
            repo: "b".into(),
        };
        let with_id = already_installed_message(&ctx, Some(42));
        assert!(with_id.contains("id `42`"));
        let without = already_installed_message(&ctx, None);
        assert!(!without.contains("id `"));
    }
}
