//! Git session context — auto-detect git repo info for system prompt injection.
//!
//! Runs git commands via `std::process::Command` to gather repo metadata:
//! remote URL, current branch, default branch, dirty state, and recent commits.

use std::process::Command;

/// Summary of git repository state for the current working directory.
#[derive(Debug, Clone, Default)]
pub struct GitContext {
    /// Remote origin URL (e.g. "git@github.com:user/repo.git"), or None if no remote.
    pub repo_url: Option<String>,
    /// Current HEAD branch name (e.g. "main", "feat/x").
    pub current_branch: Option<String>,
    /// Default branch (e.g. "main", "master") — detected from origin/HEAD.
    pub default_branch: Option<String>,
    /// Whether the working tree has uncommitted changes.
    pub is_dirty: bool,
    /// Last 5 commits as oneline strings.
    pub recent_commits: Vec<String>,
}

impl GitContext {
    /// Format the git context as a string suitable for system prompt injection.
    pub fn to_prompt_string(&self) -> String {
        let mut out = String::from("## Git Context\n");

        if let Some(ref url) = self.repo_url {
            out.push_str(&format!("Repository: {url}\n"));
        }

        if let Some(ref branch) = self.current_branch {
            out.push_str(&format!("Current branch: {branch}\n"));
        }

        if let Some(ref default) = self.default_branch {
            out.push_str(&format!("Default branch: {default}\n"));
        }

        out.push_str(&format!(
            "Working tree: {}\n",
            if self.is_dirty { "dirty" } else { "clean" }
        ));

        if !self.recent_commits.is_empty() {
            out.push_str("Recent commits:\n");
            for commit in &self.recent_commits {
                out.push_str(&format!("  {commit}\n"));
            }
        }

        out
    }
}

/// Run a git command and return trimmed stdout, or None on failure.
fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if s.is_empty() { None } else { Some(s) }
}

/// Auto-detect git repo info for the current working directory.
///
/// Returns a populated `GitContext` when inside a git repo, or a default
/// (empty) context when not. Never panics — all git failures produce
/// graceful fallbacks.
pub fn get_git_context() -> GitContext {
    // Quick check: are we in a git repo at all?
    let in_repo = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !in_repo {
        return GitContext::default();
    }

    let repo_url = git_output(&["remote", "get-url", "origin"]);

    let current_branch = git_output(&["rev-parse", "--abbrev-ref", "HEAD"]);

    // Detect default branch: try origin/HEAD symref first, fall back to
    // checking if main or master exists.
    let default_branch = git_output(&["symbolic-ref", "refs/remotes/origin/HEAD"])
        .and_then(|s| s.strip_prefix("refs/remotes/origin/").map(str::to_owned))
        .or_else(|| {
            // Fallback: check if 'main' branch exists locally
            if git_output(&["rev-parse", "--verify", "refs/heads/main"]).is_some() {
                Some("main".to_owned())
            } else if git_output(&["rev-parse", "--verify", "refs/heads/master"]).is_some() {
                Some("master".to_owned())
            } else {
                None
            }
        });

    let is_dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    let recent_commits = git_output(&["log", "--oneline", "-5"])
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default();

    GitContext {
        repo_url,
        current_branch,
        default_branch,
        is_dirty,
        recent_commits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_context_default_is_empty() {
        let ctx = GitContext::default();
        assert!(ctx.repo_url.is_none());
        assert!(ctx.current_branch.is_none());
        assert!(ctx.default_branch.is_none());
        assert!(!ctx.is_dirty);
        assert!(ctx.recent_commits.is_empty());
    }

    #[test]
    fn to_prompt_string_includes_all_fields() {
        let ctx = GitContext {
            repo_url: Some("https://github.com/user/repo.git".to_owned()),
            current_branch: Some("feat/test".to_owned()),
            default_branch: Some("main".to_owned()),
            is_dirty: true,
            recent_commits: vec![
                "abc1234 First commit".to_owned(),
                "def5678 Second commit".to_owned(),
            ],
        };
        let s = ctx.to_prompt_string();
        assert!(s.contains("## Git Context"));
        assert!(s.contains("https://github.com/user/repo.git"));
        assert!(s.contains("feat/test"));
        assert!(s.contains("Default branch: main"));
        assert!(s.contains("dirty"));
        assert!(s.contains("abc1234 First commit"));
        assert!(s.contains("def5678 Second commit"));
    }

    #[test]
    fn to_prompt_string_clean_tree() {
        let ctx = GitContext {
            is_dirty: false,
            ..Default::default()
        };
        let s = ctx.to_prompt_string();
        assert!(s.contains("clean"));
    }

    // Integration test — only meaningful inside an actual git repo.
    // This test runs inside the jfc project which is itself a git repo.
    #[test]
    fn get_git_context_returns_populated_in_repo() {
        let ctx = get_git_context();
        // We're running inside the jfc repo, so we should have a branch.
        assert!(
            ctx.current_branch.is_some(),
            "expected to find current branch in git repo"
        );
        // Should have at least some recent commits.
        assert!(
            !ctx.recent_commits.is_empty(),
            "expected recent commits in git repo"
        );
    }
}
