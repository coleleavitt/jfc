use std::path::Path;

use super::{ExecutionResult, snapshot_event_sender};

pub async fn execute_enter_plan_mode(reason: &str) -> ExecutionResult {
    let Some(tx) = snapshot_event_sender() else {
        return ExecutionResult::failure(
            "enter_plan_mode: no event sender registered (main.rs must call \
             tools::register_event_sender during startup)",
        );
    };
    let reason = reason.to_owned();
    if let Err(e) = tx
        .send(crate::runtime::EngineEvent::Frontend(
            crate::runtime::FrontendEvent::PlanModeEntered {
                reason: reason.clone(),
            },
        ))
        .await
    {
        return ExecutionResult::failure(format!("enter_plan_mode: send failed: {e}"));
    }
    ExecutionResult::success(format!(
        "Entered plan mode (reason: {})",
        if reason.is_empty() { "(none)" } else { &reason }
    ))
}

/// Whether a (already-trimmed) `SetGoal` condition *clears* the goal: an empty
/// string or one of the clear-words (`clear`/`stop`/…). This MUST match the
/// engine handler's clear test (`handle_set_goal`, which clears on
/// `is_empty() || goal::is_clear_arg`) — otherwise the tool result reports
/// "Session goal set" while the engine actually cleared it (the model then
/// believes it set a goal named "clear"). See `goal::is_clear_arg`.
fn set_goal_clears(condition: &str) -> bool {
    condition.is_empty() || crate::goal::is_clear_arg(condition)
}

/// Model-invocable `SetGoal`: emit a `GoalSet` FrontendEvent so the main loop
/// sets (or clears) the session stop-condition on a single owning task — the
/// same path `/goal` uses. An empty/clear `condition` clears the goal.
pub async fn execute_set_goal(condition: &str) -> ExecutionResult {
    let Some(tx) = snapshot_event_sender() else {
        return ExecutionResult::failure(
            "set_goal: no event sender registered (main.rs must call \
             tools::register_event_sender during startup)",
        );
    };
    let condition = condition.trim().to_owned();
    if let Err(e) = tx
        .send(crate::runtime::EngineEvent::Frontend(
            crate::runtime::FrontendEvent::GoalSet {
                condition: condition.clone(),
            },
        ))
        .await
    {
        return ExecutionResult::failure(format!("set_goal: send failed: {e}"));
    }
    // Mirror the engine handler's clear test so the success message matches what
    // actually happened: a clear-word like "clear"/"stop" clears the goal, it
    // does NOT register a goal literally named "clear".
    if set_goal_clears(&condition) {
        ExecutionResult::success("Session goal cleared.".to_owned())
    } else {
        ExecutionResult::success(format!(
            "Session goal set: \"{condition}\". The goal loop will keep you working \
             until it is met (auto-evaluated each turn). Call SetGoal with an empty \
             condition to clear it."
        ))
    }
}

// ─── EnterWorktree / ExitWorktree ──────────────────────────────────────────
//
// EnterWorktree creates the worktree (idempotent on git's side — it errors
// if it already exists, which we catch and treat as success). The agent's
// effective cwd does NOT actually change — main.rs would need to swap it
// over for that. We return a success message documenting where the worktree
// landed and what branch it's on. ExitWorktree is presently a documentation
// shim because cwd switching is out of scope for the tool layer.
pub async fn execute_enter_worktree(
    name: &str,
    branch: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    if let Err(e) = crate::worktrees::validate_name(name) {
        return ExecutionResult::failure(format!("enter_worktree: {e}"));
    }
    let repo_root = match find_repo_root(cwd) {
        Some(r) => r,
        None => {
            return ExecutionResult::failure(format!(
                "enter_worktree: {} is not inside a git repository",
                cwd.display()
            ));
        }
    };

    // If a branch override was supplied we route through `git worktree
    // add <path> <branch>` directly — `create_worktree_async` always
    // creates a new `worktree-<slug>` branch, which would clobber the
    // caller's intent.
    if let Some(branch) = branch.filter(|s| !s.is_empty()) {
        let rel_path = crate::worktrees::worktree_rel_path(name);
        let output = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo_root)
            .arg("worktree")
            .arg("add")
            .arg(&rel_path)
            .arg(branch)
            .output()
            .await;
        return match output {
            Ok(out) if out.status.success() => {
                let abs = repo_root.join(&rel_path);
                ExecutionResult::success(format!(
                    "Worktree '{name}' ready at {} on branch '{branch}'",
                    abs.display()
                ))
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                if stderr.contains("already exists") || stderr.contains("already checked out") {
                    let abs = repo_root.join(&rel_path);
                    ExecutionResult::success(format!(
                        "Worktree '{name}' already exists at {} (branch '{branch}')",
                        abs.display()
                    ))
                } else {
                    ExecutionResult::failure(format!(
                        "enter_worktree: `git worktree add` failed: {}",
                        stderr.trim()
                    ))
                }
            }
            Err(e) => ExecutionResult::failure(format!("enter_worktree: spawn failed: {e}")),
        };
    }

    match crate::worktrees::create_worktree_async(&repo_root, name).await {
        Ok(info) => ExecutionResult::success(format!(
            "Worktree '{name}' created at {} on branch '{}'",
            info.path, info.branch
        )),
        Err(e) => {
            // git emits "already exists" when a worktree with that name
            // is already registered. That's the idempotent case — return
            // success and tell the caller where it landed.
            if e.contains("already exists") || e.contains("already checked out") {
                let abs = repo_root.join(crate::worktrees::worktree_rel_path(name));
                ExecutionResult::success(format!(
                    "Worktree '{name}' already exists at {}",
                    abs.display()
                ))
            } else {
                ExecutionResult::failure(format!("enter_worktree: {e}"))
            }
        }
    }
}

pub async fn execute_exit_worktree(cwd: &Path) -> ExecutionResult {
    // The tool layer doesn't currently swap the agent's cwd; the user can
    // exit the worktree by issuing the next command in the parent repo.
    // We return an informational message rather than erroring so the model
    // can chain ExitWorktree as a no-op intent marker.
    let repo_root = find_repo_root(cwd)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| cwd.display().to_string());
    ExecutionResult::success(format!(
        "exit_worktree: cwd-switching is not yet handled by the tool layer; \
         subsequent commands will continue to run in the current cwd ({}). \
         Use the `/worktree` slash command to manually return to {repo_root}.",
        cwd.display()
    ))
}

pub fn find_repo_root(start: &Path) -> Option<std::path::PathBuf> {
    let mut cur = start;
    loop {
        // `.git` may be a directory (normal clone) OR a file containing
        // `gitdir: <path>` (linked worktrees, submodules). `exists()`
        // already accepts both, so this naturally covers gitfile layouts.
        if cur.join(".git").exists() {
            return Some(cur.to_path_buf());
        }
        cur = cur.parent()?;
    }
}

// ─── NotebookRead / NotebookEdit ──────────────────────────────────────────
//
// Jupyter `.ipynb` files are JSON documents with a `cells` array. Each
// cell has `id` (nbformat 4.5+), `cell_type`, `source` (string or array
// of strings), and code cells additionally have `outputs`. We parse,
// splice, and write back without round-tripping through nbformat — keeps
// the tool dependency-free.

#[cfg(test)]
mod tests {
    use super::set_goal_clears;

    // The tool's success-message branch MUST agree with the engine handler's
    // clear test (handle_set_goal: empty || goal::is_clear_arg). Otherwise
    // `SetGoal("clear")` clears the goal but the tool reports it as set — the
    // exact regression this guards.
    #[test]
    fn clear_words_clear_the_goal_normal() {
        for word in ["", "clear", "stop", "off", "reset", "none", "cancel"] {
            assert!(
                set_goal_clears(word),
                "{word:?} should clear the goal (parity with handle_set_goal)"
            );
        }
    }

    // Real conditions must NOT be treated as a clear — they register a goal.
    #[test]
    fn real_conditions_set_the_goal_normal() {
        for cond in ["all tests pass", "the build is green", "clear the backlog"] {
            assert!(
                !set_goal_clears(cond),
                "{cond:?} is a real goal condition, not a clear"
            );
        }
    }

    // `execute_set_goal` trims the raw arg and `is_clear_arg` lowercases before
    // matching, so mixed-case and whitespace-padded clear-words must clear too.
    // Replicate that trim→classify chain (the tool does `condition.trim()` then
    // `set_goal_clears`) so the end-to-end path is covered, not just lowercase
    // post-trim forms.
    #[test]
    fn clear_words_clear_regardless_of_case_and_padding_robust() {
        for raw in ["CLEAR", "Stop", " off ", "\tReset\n", "NONE", "  cancel"] {
            assert!(
                set_goal_clears(raw.trim()),
                "{raw:?} (after trim) should clear the goal"
            );
        }
    }
}
