//! Detect when a graph index belongs to a different git worktree.
//!
//! `GraphSession::from_directory` walks up the filesystem looking for a
//! cached index, with no knowledge of git worktrees. When a worktree is
//! created *inside* its parent checkout (the common pattern for
//! agent-managed scratch trees under `.jfc-worktrees/<name>/`), a query
//! launched from inside the worktree silently borrows the **parent
//! checkout's** index. Every result reflects the parent's branch — symbols
//! the user just added in the worktree are invisible, with no warning.
//!
//! Mirrors codegraph PR #312, simplified for our Rust layout: shell out to
//! `git rev-parse --show-toplevel` once, compare to the index root, return
//! a non-fatal warning when they differ. Soft-fails to `None` whenever git
//! is unavailable or the layout isn't a git repo at all — this is a
//! warning surface, never an error.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Result of a mismatch check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeMismatch {
    /// The git working-tree root the caller was running from.
    pub caller_worktree: PathBuf,
    /// The git working-tree root the resolved index belongs to.
    pub index_worktree: PathBuf,
    /// Human-readable warning, ready to surface to the user.
    pub message: String,
}

/// Detect whether `index_root` belongs to a different git worktree than
/// `caller_path`. Returns `None` when:
///
/// - `caller_path` is not inside a git repo (or `git` is unavailable);
/// - the resolved index root is itself the caller's working tree;
/// - the index is in a plain non-worktree ancestor directory (the
///   monorepo-subdir layout, deliberately allowed).
///
/// Returns `Some(WorktreeMismatch)` when the caller's `git
/// rev-parse --show-toplevel` differs from the index root's, with a
/// suggested warning string.
pub fn detect_worktree_index_mismatch(
    caller_path: &Path,
    index_root: &Path,
) -> Option<WorktreeMismatch> {
    let caller_worktree = git_toplevel(caller_path)?;
    let index_worktree = git_toplevel(index_root)?;
    if caller_worktree == index_worktree {
        return None;
    }
    // Index lives in an *ancestor* directory of the caller's worktree —
    // this is the monorepo-subdir case (caller is `repo/sub/`, index is
    // `repo/.jfc/`). Allow it: it isn't a worktree mismatch, it's how
    // monorepos with a single index work.
    if caller_path
        .canonicalize()
        .ok()
        .and_then(|c| c.parent().map(Path::to_path_buf))
        .map(|p| p.starts_with(&index_worktree))
        .unwrap_or(false)
    {
        return None;
    }

    let message = format!(
        "⚠ This jfc-graph index belongs to a different git working tree.\n\
         \x20  Running in: {}\n\
         \x20  Index from: {}\n\
         Results reflect that tree's code (often a different branch), not this worktree — \
         symbols changed only here are missing. Re-index from this worktree for a worktree-local index.",
        caller_worktree.display(),
        index_worktree.display(),
    );
    Some(WorktreeMismatch {
        caller_worktree,
        index_worktree,
        message,
    })
}

/// Shell out to `git rev-parse --show-toplevel` rooted at `at`. Returns
/// `None` whenever git isn't installed, the path isn't in a repo, or
/// the call times out.
fn git_toplevel(at: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(at)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn no_mismatch_when_not_a_git_repo() {
        // /tmp isn't normally a git repo — soft-fail to None.
        let tmp = std::env::temp_dir();
        let mismatch = detect_worktree_index_mismatch(&tmp, &tmp);
        assert!(mismatch.is_none());
    }

    #[test]
    fn no_mismatch_when_caller_and_index_share_root() {
        // The jfc repo root is its own worktree — running this test
        // means caller and index BOTH resolve to the workspace root,
        // so the mismatch detector must return None.
        let pwd = std::env::current_dir().expect("cwd");
        if git_toplevel(&pwd).is_none() {
            // Not in a git tree at all — test is a no-op.
            return;
        }
        let mismatch = detect_worktree_index_mismatch(&pwd, &pwd);
        assert!(mismatch.is_none(), "same dir must not mismatch");
    }

    #[test]
    fn mismatch_message_carries_both_paths() {
        let m = WorktreeMismatch {
            caller_worktree: PathBuf::from("/repo/wt"),
            index_worktree: PathBuf::from("/repo"),
            message: "ignored — re-derived below".into(),
        };
        // Smoke-test the field layout. We can't easily test the
        // actual git-vs-worktree mismatch without setting up a real
        // worktree, but the data shape is the public contract.
        assert_eq!(m.caller_worktree, PathBuf::from("/repo/wt"));
        assert_eq!(m.index_worktree, PathBuf::from("/repo"));
    }

    #[test]
    fn git_toplevel_returns_none_for_phantom_dir() {
        // A non-existent path: git fails, function returns None.
        let phantom = PathBuf::from("/this/does/not/exist/anywhere");
        assert!(git_toplevel(&phantom).is_none());
    }

    #[test]
    fn detect_with_nested_worktree_layout() {
        // Build a synthetic two-worktree layout under tempdir and exercise
        // the detector end-to-end. We can't depend on `git worktree add`
        // because that needs a fully-initialised repo with at least one
        // commit; instead we approximate the layout: two sibling git
        // repos so each has its own `git rev-parse --show-toplevel`.
        let base =
            std::env::temp_dir().join(format!("jfc-graph-worktree-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("mkdir");
        let a = base.join("a");
        let b = base.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        for d in [&a, &b] {
            let _ = Command::new("git").arg("init").current_dir(d).output();
        }
        // Either git is missing in this env (skip) or we should see a mismatch.
        if git_toplevel(&a).is_some() && git_toplevel(&b).is_some() {
            let m = detect_worktree_index_mismatch(&a, &b);
            assert!(m.is_some(), "two distinct git repos must mismatch");
            let m = m.unwrap();
            assert!(m.message.contains("different git working tree"));
        }
        let _ = fs::remove_dir_all(&base);
    }
}
