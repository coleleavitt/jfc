//! Stable cross-machine project identity.
//!
//! A project's `project_key` must be the *same* on every clone/checkout so a
//! lesson learned in one place is recalled in the same project elsewhere.
//! We derive it from the normalized git `origin` remote URL when available
//! (clone-location independent), falling back to the canonical repo-root path
//! when there is no remote.

use std::path::Path;
use std::process::Command;

/// Compute a stable project key for the repo containing `dir`.
///
/// Preference order:
/// 1. Normalized `git remote get-url origin` — identical across clones.
/// 2. The canonical repo root path — stable on one machine when there's no
///    remote.
/// 3. The given dir as-is — last resort.
///
/// The returned key is a short hex digest so it's safe as a SQL value and
/// doesn't leak a full path/URL into the row.
pub fn project_key(dir: &Path) -> String {
    let basis = remote_url(dir)
        .map(|url| format!("remote:{}", normalize_remote(&url)))
        .or_else(|| repo_root(dir).map(|root| format!("root:{}", root)))
        .unwrap_or_else(|| format!("dir:{}", dir.display()));
    digest(&basis)
}

fn remote_url(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!url.is_empty()).then_some(url)
}

fn repo_root(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!root.is_empty()).then_some(root)
}

/// Normalize a git remote URL so `git@github.com:o/r.git`,
/// `https://github.com/o/r.git`, and `https://github.com/o/r` all map to the
/// same key (`github.com/o/r`).
pub fn normalize_remote(url: &str) -> String {
    let mut s = url.trim().to_ascii_lowercase();
    // Strip scheme.
    for prefix in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_owned();
            break;
        }
    }
    // scp-like `git@host:owner/repo` → `host/owner/repo`.
    if let Some(rest) = s.strip_prefix("git@") {
        s = rest.replacen(':', "/", 1);
    } else if let Some(at) = s.find('@') {
        // `user@host/...` → drop the `user@` credentials part.
        s = s[at + 1..].to_owned();
    }
    // Drop a trailing `.git` and any trailing slashes.
    s = s.trim_end_matches('/').to_owned();
    if let Some(stripped) = s.strip_suffix(".git") {
        s = stripped.to_owned();
    }
    s
}

/// Short, stable hex digest (uuid v5 over a fixed namespace — no extra hashing
/// dep, deterministic across machines).
fn digest(basis: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, basis.as_bytes())
        .simple()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_remote_unifies_url_forms_normal() {
        let want = "github.com/owner/repo";
        for url in [
            "git@github.com:owner/repo.git",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo",
            "ssh://git@github.com/owner/repo.git",
            "https://token@github.com/owner/repo.git",
            "git@github.com:owner/repo.git/",
        ] {
            assert_eq!(normalize_remote(url), want, "url={url}");
        }
    }

    #[test]
    fn project_key_is_deterministic_and_remote_independent_of_path_normal() {
        // Same normalized remote → same key, regardless of the basis string's
        // source. We can't run git in a unit test reliably, so assert the digest
        // is a pure function of its basis.
        let a = digest("remote:github.com/owner/repo");
        let b = digest("remote:github.com/owner/repo");
        let c = digest("remote:github.com/owner/other");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 32); // simple uuid hex
    }
}
