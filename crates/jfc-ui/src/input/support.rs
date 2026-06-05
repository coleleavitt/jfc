//! Project-support URLs — single source of truth for the canonical
//! GitHub repository, release page, and bug-report endpoint.
//!
//! Centralized here because the same handful of URLs were previously
//! hard-coded in four different places (`/feedback`, `/bug`, `/upgrade`,
//! `/release-notes`), and three of them pointed at the wrong owner
//! (`github.com/RustProjects/jfc` and `github.com/anthropics/jfc`),
//! which is what the user actually saw — `/feedback` opened a 404 page.
//!
//! Frontier CLIs (`gh issue create --web`, Claude Code's `/bug`) follow
//! the same pattern: build a `new` URL with the standard query params
//! GitHub honors (`title`, `body`, `labels`, `template`) so the user
//! lands in a pre-populated form with their environment / session
//! context already attached. See:
//!   <https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/creating-an-issue#creating-an-issue-from-a-url-query>

/// Canonical project owner. Matches the `origin` git remote
/// (`git@github.com:coleleavitt/jfc.git`).
pub(crate) const REPO_OWNER: &str = "coleleavitt";
/// Canonical project repository name.
pub(crate) const REPO_NAME: &str = "jfc";

/// Browser URL of the project repository root.
pub(crate) fn repo_url() -> String {
    format!("https://github.com/{REPO_OWNER}/{REPO_NAME}")
}

/// Releases / changelog page (used by `/release-notes` when the
/// bundled CHANGELOG.md isn't readable).
pub(crate) fn releases_url() -> String {
    format!("{}/releases", repo_url())
}

/// `cargo install --git <url>` source — same root URL minus the
/// trailing slash, suitable for placing directly after `--git`.
pub(crate) fn cargo_install_git_url() -> String {
    repo_url()
}

/// Build a bug-report URL with title + body pre-populated via GitHub's
/// standard issue-template query parameters. Both fields are URL-
/// encoded per RFC 3986 form-encoding — the same encoding `gh issue
/// create --web` uses.
///
/// The body is intentionally a Markdown template that mirrors what
/// frontier CLIs ship: session id, provider/model, mode, and a
/// reproduction section. The user lands on `issues/new` with the
/// form already filled in.
pub(crate) fn bug_report_url(title: &str, body: &str) -> String {
    format!(
        "{}/issues/new?title={}&body={}&labels=bug",
        repo_url(),
        urlencode(title),
        urlencode(body),
    )
}

/// RFC 3986 percent-encoding for query-string values. We can't pull in
/// `urlencoding` just for this — keep the encoder local and minimal.
/// Encodes everything except the unreserved set `A-Z a-z 0-9 - _ . ~`.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: owner / repo / canonical URL stays consistent with the
    // git remote so a refactor that breaks one of these surfaces in CI.
    #[test]
    fn repo_url_points_at_real_remote_normal() {
        assert_eq!(repo_url(), "https://github.com/coleleavitt/jfc");
        assert_eq!(
            releases_url(),
            "https://github.com/coleleavitt/jfc/releases"
        );
    }

    // Normal: bug URL puts title + body into the standard GitHub query
    // params with proper percent-encoding.
    #[test]
    fn bug_report_url_encodes_title_and_body_normal() {
        let u = bug_report_url("crash on /feedback", "hello world\nsecond line");
        assert!(u.starts_with("https://github.com/coleleavitt/jfc/issues/new?"));
        assert!(u.contains("title=crash%20on%20%2Ffeedback"));
        assert!(u.contains("body=hello%20world%0Asecond%20line"));
        assert!(u.ends_with("&labels=bug"));
    }

    // Robust: reserved + multibyte chars must percent-encode, not pass through.
    #[test]
    fn urlencode_escapes_reserved_and_unicode_robust() {
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("café"), "caf%C3%A9");
        assert_eq!(urlencode("ABCabc012-_.~"), "ABCabc012-_.~");
    }
}
