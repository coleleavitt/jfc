//! Git worktree management for jfc.
//!
//! This is jfc-side worktree control — a thin wrapper around `git worktree`
//! plumbing surfaced as `/worktree create|list|remove|switch` slash commands.
//! It is intentionally separate from the v126 `Agent({isolation:'worktree'})`
//! pathway (which spawns subagents in their own checkouts via cli.js
//! teammate-spawn flow). The use case here is the user/model wanting an
//! isolated branch for a risky multi-file change without trampling the main
//! checkout.
//!
//! Layout: created worktrees live at `<repo_root>/.jfc-worktrees/<name>` and
//! check out a fresh branch `jfc/<name>`. Removing a worktree only deletes
//! the working tree directory; the branch itself is left intact so the work
//! is recoverable via `git switch jfc/<name>` from any other checkout.
//!
//! The shell-out functions (`list_worktrees`, `create_worktree`,
//! `remove_worktree`) intentionally have no unit tests — they invoke real
//! `git` and depend on an actual repository on disk. Coverage for them lives
//! in manual / integration testing. The pure helpers (`validate_name`,
//! `parse_porcelain_output`) are exercised below.

use std::path::Path;
use std::process::Command;
use tokio::process::Command as TokioCommand;

/// One row from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub is_current: bool,
}

/// Validate a worktree name before it reaches `git`. Names become both a
/// directory under `.jfc-worktrees/` and the leaf of a `jfc/<name>` branch,
/// so we restrict to `[A-Za-z0-9_-]` to keep both shells and refs happy.
/// Empty input and inputs over 64 chars are rejected.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("worktree name must not be empty".to_owned());
    }
    if name.len() > 64 {
        return Err(format!(
            "worktree name must be <= 64 chars (got {})",
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "worktree name `{name}` must match [A-Za-z0-9_-] only \
             (no slashes, dots, or whitespace)"
        ));
    }
    tracing::trace!(target: "jfc::worktrees", name, "validate_name ok");
    Ok(())
}

/// Parse the porcelain output of `git worktree list --porcelain`.
///
/// Format (per git-worktree(1)): each entry is a sequence of label/value
/// lines, terminated by a blank line. The labels we care about:
///   `worktree <abs-path>`
///   `HEAD <sha>`
///   `branch refs/heads/<branch>` — present when checked out on a branch
///   `detached`                   — present when HEAD is detached
///   `bare`                       — present for the bare repo entry
///
/// We surface `branch` as `(detached)` for detached entries and `(bare)` for
/// the bare entry so the UI list never has an empty branch column.
///
/// `is_current` is left `false` here — porcelain output doesn't mark which
/// worktree is "current". Callers that need it should compare paths against
/// their own cwd.
pub fn parse_porcelain_output(s: &str) -> Vec<WorktreeInfo> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut detached = false;
    let mut bare = false;

    let flush = |out: &mut Vec<WorktreeInfo>,
                 path: &mut Option<String>,
                 branch: &mut Option<String>,
                 detached: &mut bool,
                 bare: &mut bool| {
        if let Some(p) = path.take() {
            let b = if *bare {
                "(bare)".to_owned()
            } else if *detached {
                "(detached)".to_owned()
            } else {
                branch.take().unwrap_or_default()
            };
            out.push(WorktreeInfo {
                path: p,
                branch: b,
                is_current: false,
            });
        }
        *branch = None;
        *detached = false;
        *bare = false;
    };

    for line in s.lines() {
        if line.is_empty() {
            flush(&mut out, &mut path, &mut branch, &mut detached, &mut bare);
            continue;
        }
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(p.to_owned());
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            branch = Some(b.to_owned());
        } else if let Some(b) = line.strip_prefix("branch ") {
            // Non-heads ref (e.g. refs/remotes/...). Keep the raw value.
            branch = Some(b.to_owned());
        } else if line == "detached" {
            detached = true;
        } else if line == "bare" {
            bare = true;
        }
        // `HEAD <sha>` and unknown labels are intentionally skipped.
    }
    // Final entry may not be followed by a trailing blank line.
    flush(&mut out, &mut path, &mut branch, &mut detached, &mut bare);
    out
}

/// List all worktrees registered with the repo at `repo_root`. Shells out to
/// `git worktree list --porcelain` — not unit-tested; see module docs.
pub fn list_worktrees(repo_root: &Path) -> Result<Vec<WorktreeInfo>, String> {
    tracing::debug!(target: "jfc::worktrees", ?repo_root, "list_worktrees");
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("list")
        .arg("--porcelain")
        .output()
        .map_err(|e| format!("failed to spawn `git worktree list`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`git worktree list` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries = parse_porcelain_output(&stdout);
    tracing::debug!(target: "jfc::worktrees", count = entries.len(), "list_worktrees done");
    Ok(entries)
}

/// Async variant of `list_worktrees` for use in tokio event loops.
/// Uses `tokio::process` so the subprocess spawn does not block the runtime.
pub async fn list_worktrees_async(repo_root: &Path) -> Result<Vec<WorktreeInfo>, String> {
    tracing::debug!(target: "jfc::worktrees", ?repo_root, "list_worktrees_async");
    let output = TokioCommand::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("list")
        .arg("--porcelain")
        .output()
        .await
        .map_err(|e| format!("failed to spawn `git worktree list`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`git worktree list` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries = parse_porcelain_output(&stdout);
    tracing::debug!(target: "jfc::worktrees", count = entries.len(), "list_worktrees_async done");
    Ok(entries)
}

/// Create `<repo_root>/.jfc-worktrees/<name>` checking out a fresh branch
/// `jfc/<name>`. Validates the name first; surfaces git's stderr on failure.
/// Shells out — not unit-tested; see module docs.
pub fn create_worktree(repo_root: &Path, name: &str) -> Result<WorktreeInfo, String> {
    tracing::info!(target: "jfc::worktrees", ?repo_root, name, "create_worktree");
    validate_name(name)?;
    let rel_path = format!(".jfc-worktrees/{name}");
    let branch = format!("jfc/{name}");
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("add")
        .arg(&rel_path)
        .arg("-b")
        .arg(&branch)
        .output()
        .map_err(|e| format!("failed to spawn `git worktree add`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(target: "jfc::worktrees", name, %stderr, "create_worktree failed");
        return Err(format!("`git worktree add` failed: {}", stderr.trim()));
    }
    let abs_path = repo_root.join(&rel_path).display().to_string();
    tracing::info!(target: "jfc::worktrees", name, path = %abs_path, "create_worktree ok");
    Ok(WorktreeInfo {
        path: abs_path,
        branch,
        is_current: false,
    })
}

/// Async variant of `create_worktree`.
pub async fn create_worktree_async(repo_root: &Path, name: &str) -> Result<WorktreeInfo, String> {
    tracing::info!(target: "jfc::worktrees", ?repo_root, name, "create_worktree_async");
    validate_name(name)?;
    let rel_path = format!(".jfc-worktrees/{name}");
    let branch = format!("jfc/{name}");
    let output = TokioCommand::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("add")
        .arg(&rel_path)
        .arg("-b")
        .arg(&branch)
        .output()
        .await
        .map_err(|e| format!("failed to spawn `git worktree add`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(target: "jfc::worktrees", name, %stderr, "create_worktree_async failed");
        return Err(format!("`git worktree add` failed: {}", stderr.trim()));
    }
    let abs_path = repo_root.join(&rel_path).display().to_string();
    tracing::info!(target: "jfc::worktrees", name, path = %abs_path, "create_worktree_async ok");
    Ok(WorktreeInfo {
        path: abs_path,
        branch,
        is_current: false,
    })
}

/// Remove `<repo_root>/.jfc-worktrees/<name>`. The `jfc/<name>` branch is NOT
/// deleted — the user can still recover the work by checking the branch out
/// elsewhere. Shells out — not unit-tested; see module docs.
pub fn remove_worktree(repo_root: &Path, name: &str) -> Result<(), String> {
    tracing::info!(target: "jfc::worktrees", ?repo_root, name, "remove_worktree");
    validate_name(name)?;
    let rel_path = format!(".jfc-worktrees/{name}");
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("remove")
        .arg(&rel_path)
        .output()
        .map_err(|e| format!("failed to spawn `git worktree remove`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(target: "jfc::worktrees", name, %stderr, "remove_worktree failed");
        return Err(format!("`git worktree remove` failed: {}", stderr.trim()));
    }
    tracing::info!(target: "jfc::worktrees", name, "remove_worktree ok");
    Ok(())
}

/// Async variant of `remove_worktree`.
pub async fn remove_worktree_async(repo_root: &Path, name: &str) -> Result<(), String> {
    tracing::info!(target: "jfc::worktrees", ?repo_root, name, "remove_worktree_async");
    validate_name(name)?;
    let rel_path = format!(".jfc-worktrees/{name}");
    let output = TokioCommand::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("worktree")
        .arg("remove")
        .arg(&rel_path)
        .output()
        .await
        .map_err(|e| format!("failed to spawn `git worktree remove`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(target: "jfc::worktrees", name, %stderr, "remove_worktree_async failed");
        return Err(format!("`git worktree remove` failed: {}", stderr.trim()));
    }
    tracing::info!(target: "jfc::worktrees", name, "remove_worktree_async ok");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_accepts_normal() {
        for name in ["feature-x", "x_y", "abc123", "A", "a-_-b"] {
            assert!(
                validate_name(name).is_ok(),
                "expected `{name}` to validate, got {:?}",
                validate_name(name)
            );
        }
    }

    #[test]
    fn validate_name_rejects_empty_robust() {
        let err = validate_name("").expect_err("empty name must be rejected");
        assert!(
            err.contains("empty"),
            "error message should mention empty, got: {err}"
        );
    }

    #[test]
    fn validate_name_rejects_path_traversal_robust() {
        for bad in ["../foo", "foo/bar", "foo bar", "..", ".", "a/b/c", "x\ty"] {
            assert!(
                validate_name(bad).is_err(),
                "expected `{bad}` to be rejected as invalid"
            );
        }
    }

    #[test]
    fn validate_name_rejects_long_robust() {
        let too_long: String = "a".repeat(65);
        let err = validate_name(&too_long).expect_err("65-char name must be rejected");
        assert!(
            err.contains("64"),
            "error should reference the 64-char cap, got: {err}"
        );
        // Boundary: exactly 64 chars must still be accepted.
        let ok: String = "a".repeat(64);
        assert!(
            validate_name(&ok).is_ok(),
            "exactly 64 chars should be accepted"
        );
    }

    #[test]
    fn parse_worktree_porcelain_basic_normal() {
        let blob = "worktree /a\nHEAD abc\nbranch refs/heads/main\n\
                    \n\
                    worktree /b\nHEAD def\nbranch refs/heads/feat\n";
        let got = parse_porcelain_output(blob);
        assert_eq!(got.len(), 2, "expected 2 entries, got {got:?}");
        assert_eq!(got[0].path, "/a");
        assert_eq!(got[0].branch, "main");
        assert_eq!(got[1].path, "/b");
        assert_eq!(got[1].branch, "feat");
    }

    #[test]
    fn parse_worktree_porcelain_handles_detached_robust() {
        let blob = "worktree /a\nHEAD abc\ndetached\n";
        let got = parse_porcelain_output(blob);
        assert_eq!(got.len(), 1, "detached entry should still produce a row");
        assert_eq!(got[0].path, "/a");
        assert_eq!(
            got[0].branch, "(detached)",
            "detached HEAD should surface as `(detached)`"
        );
    }

    #[test]
    fn parse_worktree_porcelain_empty_input_robust() {
        assert!(
            parse_porcelain_output("").is_empty(),
            "empty input must produce empty vec"
        );
        assert!(
            parse_porcelain_output("\n\n\n").is_empty(),
            "whitespace-only input must produce empty vec"
        );
    }

    // Robust: bare-repo entries surface as `(bare)` so the UI's branch
    // column never has an empty cell.
    #[test]
    fn parse_worktree_porcelain_handles_bare_robust() {
        let blob = "worktree /a/bare\nbare\n";
        let got = parse_porcelain_output(blob);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/a/bare");
        assert_eq!(got[0].branch, "(bare)");
    }

    // Robust: a non-heads ref path on `branch` (e.g. refs/remotes/origin/main)
    // still gets surfaced verbatim instead of being dropped.
    #[test]
    fn parse_worktree_porcelain_keeps_non_heads_branch_robust() {
        let blob = "worktree /a\nHEAD abc\nbranch refs/remotes/origin/main\n";
        let got = parse_porcelain_output(blob);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].branch, "refs/remotes/origin/main");
    }

    // ── Live-git fixture tests (use #[ignore] when git isn't available) ──

    /// Initialize a fresh git repo in `dir` so the worktree commands have
    /// something to operate on. Returns true on success, false otherwise so
    /// callers can skip without failing.
    fn init_git_repo(dir: &Path) -> bool {
        let ok = Command::new("git")
            .args(["init", "-q", "--initial-branch=main"])
            .current_dir(dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return false;
        }
        // Set local user.name/email so commit doesn't error in CI sandboxes
        // that lack a global config.
        let _ = Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(dir)
            .status();
        // Need a commit on the branch — `git worktree add -b foo` requires a
        // valid base ref.
        std::fs::write(dir.join("seed.txt"), "x").ok();
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status();
        Command::new("git")
            .args(["commit", "-q", "-m", "seed"])
            .current_dir(dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Build a unique tempdir for the test fixture. Skip the test if the
    /// platform's temp dir is unwritable.
    fn fixture_tempdir(label: &str) -> Option<std::path::PathBuf> {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "jfc_worktrees_{label}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).ok()?;
        Some(p)
    }

    // Normal: create a worktree against a fresh fixture repo, then list and
    // remove it. Skips silently if `git` isn't on PATH (CI-safe).
    #[test]
    fn create_list_remove_worktree_round_trip_normal() {
        let Some(repo) = fixture_tempdir("create") else {
            eprintln!("temp dir unavailable, skipping");
            return;
        };
        if !init_git_repo(&repo) {
            eprintln!("git unavailable, skipping create_worktree round-trip");
            let _ = std::fs::remove_dir_all(&repo);
            return;
        }
        let info = create_worktree(&repo, "feat-x").expect("create_worktree should succeed");
        assert_eq!(info.branch, "jfc/feat-x");
        assert!(info.path.contains(".jfc-worktrees"));
        // The new worktree directory must exist on disk.
        let wt_dir = repo.join(".jfc-worktrees").join("feat-x");
        assert!(
            wt_dir.exists(),
            "worktree dir was not created at {wt_dir:?}"
        );

        let listed = list_worktrees(&repo).expect("list_worktrees should succeed");
        assert!(
            listed.iter().any(|w| w.branch == "jfc/feat-x"),
            "expected jfc/feat-x in {listed:?}"
        );

        // Remove and confirm it's gone from the listing.
        remove_worktree(&repo, "feat-x").expect("remove_worktree should succeed");
        let after = list_worktrees(&repo).expect("list after remove");
        assert!(
            !after.iter().any(|w| w.branch == "jfc/feat-x"),
            "expected jfc/feat-x NOT in {after:?}"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    // Robust: an invalid worktree name short-circuits in validate_name
    // before git ever runs — no tempdir needed.
    #[test]
    fn create_worktree_invalid_name_robust() {
        let err = create_worktree(Path::new("/tmp"), "../traverse")
            .expect_err("invalid name must be rejected");
        assert!(
            err.contains("must match") || err.contains("traversal") || err.contains("/"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn remove_worktree_invalid_name_robust() {
        let err = remove_worktree(Path::new("/tmp"), "").expect_err("empty name must be rejected");
        assert!(err.contains("empty"), "unexpected error: {err}");
    }

    // Robust: duplicate creation attempts surface git's stderr instead of
    // panicking. Skips silently when git isn't available.
    #[test]
    fn create_worktree_duplicate_errors_robust() {
        let Some(repo) = fixture_tempdir("dup") else {
            return;
        };
        if !init_git_repo(&repo) {
            let _ = std::fs::remove_dir_all(&repo);
            return;
        }
        let _ = create_worktree(&repo, "feat-dup").expect("first create");
        let err = create_worktree(&repo, "feat-dup")
            .expect_err("second create with same name should fail");
        assert!(
            err.contains("git worktree add") || err.contains("already") || err.contains("exists"),
            "expected git error message, got: {err}"
        );
        // Cleanup.
        let _ = remove_worktree(&repo, "feat-dup");
        let _ = std::fs::remove_dir_all(&repo);
    }

    // Robust: list_worktrees against a non-repo directory surfaces git's
    // stderr as Err, not a panic.
    #[test]
    fn list_worktrees_non_repo_errors_robust() {
        let Some(dir) = fixture_tempdir("notrepo") else {
            return;
        };
        // Don't `git init` — this is the negative case.
        let result = list_worktrees(&dir);
        // Either err (git installed, no repo) or err (git missing) — both
        // are Err and neither should panic.
        assert!(
            result.is_err(),
            "non-repo directory should yield Err, got {result:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tmux integration for worktree agents
// ─────────────────────────────────────────────────────────────────────────────

/// Tmux session naming convention for jfc worktree agents.
pub fn tmux_session_name(agent_name: &str) -> String {
    format!("jfc-{}", agent_name.replace(' ', "-").to_lowercase())
}

/// Check if tmux is available on the system.
pub fn tmux_available() -> bool {
    Command::new("tmux").arg("-V").output().is_ok()
}

/// Create a tmux session for a worktree agent.
/// The session runs in the worktree directory and shows the agent's log.
pub fn create_tmux_session(agent_name: &str, worktree_path: &Path) -> Result<String, String> {
    let session_name = tmux_session_name(agent_name);

    // Check if session already exists
    let check = Command::new("tmux")
        .args(["has-session", "-t", &session_name])
        .output()
        .map_err(|e| format!("tmux not available: {e}"))?;

    if check.status.success() {
        return Err(format!("tmux session '{session_name}' already exists"));
    }

    // Create new detached session in the worktree directory
    let result = Command::new("tmux")
        .args([
            "new-session",
            "-d", // detached
            "-s",
            &session_name, // session name
            "-c",
            &worktree_path.to_string_lossy(), // working directory
        ])
        .output()
        .map_err(|e| format!("Failed to create tmux session: {e}"))?;

    if !result.status.success() {
        return Err(format!(
            "tmux new-session failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }

    // Set status bar to show agent info
    let _ = Command::new("tmux")
        .args([
            "set-option",
            "-t",
            &session_name,
            "status-left",
            &format!(" 🤖 {agent_name} "),
        ])
        .output();

    let _ = Command::new("tmux")
        .args([
            "set-option",
            "-t",
            &session_name,
            "status-style",
            "bg=#1a1b26,fg=#7aa2f7",
        ])
        .output();

    Ok(session_name)
}

/// Create a split pane in an existing tmux session for log tailing.
pub fn tmux_add_log_pane(session_name: &str, log_path: &Path) -> Result<(), String> {
    let result = Command::new("tmux")
        .args([
            "split-window",
            "-t",
            session_name,
            "-v", // vertical split
            "-l",
            "30%", // 30% height for logs
            "-d",  // don't switch focus
            &format!("tail -f {}", log_path.to_string_lossy()),
        ])
        .output()
        .map_err(|e| format!("Failed to split tmux pane: {e}"))?;

    if !result.status.success() {
        return Err(format!(
            "tmux split-window failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}

/// Attach to a tmux session (blocks until detach).
pub fn tmux_attach(session_name: &str) -> Result<(), String> {
    let result = Command::new("tmux")
        .args(["attach-session", "-t", session_name])
        .status()
        .map_err(|e| format!("Failed to attach: {e}"))?;

    if !result.success() {
        return Err("tmux attach failed".to_string());
    }
    Ok(())
}

/// List all jfc-related tmux sessions.
pub fn list_tmux_sessions() -> Result<Vec<String>, String> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .map_err(|e| format!("tmux not available: {e}"))?;

    if !output.status.success() {
        // No server running = no sessions
        return Ok(Vec::new());
    }

    let sessions: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|s| s.starts_with("jfc-"))
        .map(String::from)
        .collect();

    Ok(sessions)
}

/// Kill a tmux session.
pub fn kill_tmux_session(session_name: &str) -> Result<(), String> {
    let result = Command::new("tmux")
        .args(["kill-session", "-t", session_name])
        .output()
        .map_err(|e| format!("Failed to kill session: {e}"))?;

    if !result.status.success() {
        return Err(format!(
            "tmux kill-session failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}

/// Create a full worktree agent setup: worktree + tmux session + log pane.
pub fn create_agent_worktree_with_tmux(
    repo_root: &Path,
    agent_name: &str,
    log_path: Option<&Path>,
) -> Result<(WorktreeInfo, String), String> {
    // Create the worktree
    let worktree = create_worktree(repo_root, agent_name)?;

    // Create tmux session in the worktree
    let session_name = create_tmux_session(agent_name, Path::new(&worktree.path))?;

    // Add log pane if log path provided
    if let Some(log) = log_path {
        let _ = tmux_add_log_pane(&session_name, log);
    }

    Ok((worktree, session_name))
}

/// Clean up agent worktree + tmux session.
pub fn cleanup_agent_worktree_with_tmux(repo_root: &Path, agent_name: &str) -> Result<(), String> {
    let session_name = tmux_session_name(agent_name);

    // Kill tmux session (ignore errors — might not exist)
    let _ = kill_tmux_session(&session_name);

    // Remove worktree
    remove_worktree(repo_root, agent_name)?;

    Ok(())
}
