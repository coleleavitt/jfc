//! Sprint boundary detection for long-running agentic sessions.
//!
//! The "stub problem": an agent given N large tasks hits context/token limits
//! and produces incomplete work or stubs instead of properly finishing each
//! task. This module detects when the agent is approaching its limits and
//! triggers a graceful handoff:
//!
//! 1. Commit whatever is actually complete to git
//! 2. Mark current task as "in_progress" with a progress note
//! 3. Update the project-level task file with remaining work
//! 4. Generate a handoff summary for the next session
//!
//! Integrates with `compact::compact_level` for context pressure and
//! `app.last_usage_input` for actual token counts from the API.

use std::path::{Path, PathBuf};

use crate::compact::CompactLevel;

/// Percentage of context window at which we start warning about sprint limits.
const SPRINT_WARN_PCT: f64 = 0.70;
/// Percentage at which we actively suggest wrapping up the current task.
const SPRINT_HANDOFF_PCT: f64 = 0.85;

/// Sprint boundary state. Injected into the system prompt so the model
/// knows its budget and can plan accordingly.
#[derive(Debug, Clone)]
pub struct SprintBudget {
    /// Total context window in tokens for the current model.
    pub context_window: usize,
    /// Current estimated token usage (input tokens from last API response).
    pub current_tokens: usize,
    /// Percentage of context consumed (0.0 - 1.0).
    pub utilization: f64,
    /// Whether the model should start wrapping up.
    pub should_handoff: bool,
    /// Human-readable budget status for system prompt injection.
    pub status_line: String,
}

impl SprintBudget {
    /// Compute the current sprint budget from app state.
    pub fn compute(current_tokens: usize, context_window: usize) -> Self {
        let utilization = if context_window > 0 {
            current_tokens as f64 / context_window as f64
        } else {
            0.0
        };
        let should_handoff = utilization >= SPRINT_HANDOFF_PCT;
        let status_line = if should_handoff {
            format!(
                "⚠️ CONTEXT BUDGET: {:.0}% used ({}/{} tokens). \
                 You are approaching context limits. STOP creating new work. \
                 Commit your current progress, update the task file with \
                 remaining work, and provide a handoff summary.",
                utilization * 100.0,
                current_tokens,
                context_window
            )
        } else if utilization >= SPRINT_WARN_PCT {
            format!(
                "Context budget: {:.0}% used ({}/{} tokens). \
                 Plan to wrap up the current task soon.",
                utilization * 100.0,
                current_tokens,
                context_window
            )
        } else {
            format!(
                "Context budget: {:.0}% used ({}/{} tokens).",
                utilization * 100.0,
                current_tokens,
                context_window
            )
        };

        Self {
            context_window,
            current_tokens,
            utilization,
            should_handoff,
            status_line,
        }
    }

    /// Generate the system prompt section that tells the model about its
    /// sprint budget. Only included when utilization > 50% to avoid noise.
    pub fn system_prompt_section(&self) -> Option<String> {
        if self.utilization < 0.50 {
            return None;
        }
        Some(format!(
            "\n## Sprint Budget\n{}\n",
            self.status_line
        ))
    }
}

/// Handoff summary written when a session ends (gracefully or due to limits).
/// This is what the NEXT session reads to know where to pick up.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HandoffSummary {
    /// When this handoff was generated.
    pub timestamp: String,
    /// What was accomplished this session.
    pub completed_work: Vec<String>,
    /// What was in progress when the session ended.
    pub in_progress: Vec<String>,
    /// What remains to be done (from the task file).
    pub remaining_tasks: Vec<String>,
    /// Key decisions made this session.
    pub decisions: Vec<String>,
    /// Any blockers or issues encountered.
    pub blockers: Vec<String>,
}

impl HandoffSummary {
    /// Write the summary to `.jfc/session_summaries/{timestamp}.md`.
    pub fn write_to_disk(&self, git_root: &Path) -> std::io::Result<PathBuf> {
        let dir = git_root.join(".jfc").join("session_summaries");
        std::fs::create_dir_all(&dir)?;
        let filename = format!("{}.md", self.timestamp);
        let path = dir.join(&filename);

        let mut content = String::new();
        content.push_str(&format!("# Session Handoff — {}\n\n", self.timestamp));

        if !self.completed_work.is_empty() {
            content.push_str("## Completed\n");
            for item in &self.completed_work {
                content.push_str(&format!("- {}\n", item));
            }
            content.push('\n');
        }

        if !self.in_progress.is_empty() {
            content.push_str("## In Progress (pick up here)\n");
            for item in &self.in_progress {
                content.push_str(&format!("- {}\n", item));
            }
            content.push('\n');
        }

        if !self.remaining_tasks.is_empty() {
            content.push_str("## Remaining\n");
            for item in &self.remaining_tasks {
                content.push_str(&format!("- {}\n", item));
            }
            content.push('\n');
        }

        if !self.decisions.is_empty() {
            content.push_str("## Decisions Made\n");
            for item in &self.decisions {
                content.push_str(&format!("- {}\n", item));
            }
            content.push('\n');
        }

        if !self.blockers.is_empty() {
            content.push_str("## Blockers\n");
            for item in &self.blockers {
                content.push_str(&format!("- {}\n", item));
            }
            content.push('\n');
        }

        std::fs::write(&path, content)?;
        Ok(path)
    }

    /// Read the most recent handoff summary from disk (if any).
    pub fn read_latest(git_root: &Path) -> Option<String> {
        let dir = git_root.join(".jfc").join("session_summaries");
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map_or(false, |ext| ext == "md")
            })
            .collect();
        // Sort by filename (which contains timestamp) descending.
        entries.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        let latest = entries.first()?;
        std::fs::read_to_string(latest.path()).ok()
    }
}

/// Check if the current context pressure warrants a sprint boundary warning.
/// Returns the compact level for UI/logging purposes.
pub fn check_sprint_pressure(current_tokens: usize, context_window: usize) -> CompactLevel {
    crate::compact::compact_level(current_tokens, context_window)
}

/// Auto-commit progress when a sprint boundary is hit. This prevents losing
/// work when a session is about to end due to context limits.
///
/// Steps:
/// 1. `git add -A` all tracked changes
/// 2. `git commit` with a sprint-boundary message
/// 3. Write a handoff summary
///
/// Returns Ok(commit_hash) on success, Err(reason) on failure.
/// Does NOT commit if working tree is clean.
pub fn auto_commit_sprint_progress(
    git_root: &Path,
    task_store: &jfc_session::TaskStore,
) -> Result<String, String> {
    use jfc_session::DeletedFilter;

    // Check if there are changes to commit
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(git_root)
        .output()
        .map_err(|e| format!("git status failed: {e}"))?;
    let status_output = String::from_utf8_lossy(&status.stdout);
    if status_output.trim().is_empty() {
        return Err("working tree clean — nothing to commit".to_string());
    }

    // Stage all changes
    let add = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(git_root)
        .output()
        .map_err(|e| format!("git add failed: {e}"))?;
    if !add.status.success() {
        return Err(format!(
            "git add -A failed: {}",
            String::from_utf8_lossy(&add.stderr)
        ));
    }

    // Build commit message from task store state
    let tasks = task_store.list(DeletedFilter::Exclude);
    let in_progress: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == jfc_session::TaskStatus::InProgress)
        .map(|t| t.subject.as_str())
        .collect();
    let completed: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == jfc_session::TaskStatus::Completed)
        .map(|t| t.subject.as_str())
        .collect();

    let msg = format!(
        "wip(sprint): auto-commit at context boundary\n\n\
         In progress: {}\n\
         Completed this session: {}",
        if in_progress.is_empty() {
            "(none)".to_string()
        } else {
            in_progress.join(", ")
        },
        if completed.is_empty() {
            "(none)".to_string()
        } else {
            completed.join(", ")
        },
    );

    let commit = std::process::Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(git_root)
        .output()
        .map_err(|e| format!("git commit failed: {e}"))?;
    if !commit.status.success() {
        return Err(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        ));
    }

    // Get the commit hash
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(git_root)
        .output()
        .map_err(|e| format!("git rev-parse failed: {e}"))?;
    let hash_str = String::from_utf8_lossy(&hash.stdout).trim().to_string();

    // Write handoff summary
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let summary = HandoffSummary {
        timestamp: timestamp.clone(),
        completed_work: completed.iter().map(|s| s.to_string()).collect(),
        in_progress: in_progress.iter().map(|s| s.to_string()).collect(),
        remaining_tasks: tasks
            .iter()
            .filter(|t| t.status == jfc_session::TaskStatus::Pending)
            .map(|t| t.subject.clone())
            .collect(),
        decisions: vec![format!(
            "Sprint boundary hit — auto-committed at {hash_str}"
        )],
        blockers: vec![],
    };
    if let Err(e) = summary.write_to_disk(git_root) {
        tracing::warn!(
            target: "jfc::sprint",
            error = %e,
            "auto_commit_sprint_progress: failed to write handoff summary"
        );
    }

    // Mark in-progress tasks with a note about the checkpoint
    for task in &tasks {
        if task.status == jfc_session::TaskStatus::InProgress {
            let _ = task_store.update(
                task.id.as_str(),
                jfc_session::TaskPatch {
                    description: Some(format!(
                        "{}\n\n[Sprint checkpoint at {}]",
                        task.description, hash_str
                    )),
                    ..Default::default()
                },
            );
        }
    }

    tracing::info!(
        target: "jfc::sprint",
        commit = %hash_str,
        in_progress_count = in_progress.len(),
        completed_count = completed.len(),
        "auto_commit_sprint_progress: committed at sprint boundary"
    );

    Ok(hash_str)
}

// ─── Evaluator / Reviewer Pass ───────────────────────────────────────────────

/// Patterns that indicate incomplete/stub work. If any of these appear in
/// recently-modified files, the evaluator rejects the task completion.
const STUB_PATTERNS: &[&str] = &[
    "unimplemented!()",
    "todo!()",
    "todo!(\"",
    "// TODO",
    "// FIXME",
    "// STUB",
    "// PLACEHOLDER",
    "/* TODO",
    "/* FIXME",
    "panic!(\"not implemented",
    "panic!(\"not yet implemented",
];

/// Result of evaluating a task's work output.
#[derive(Debug, Clone)]
pub struct EvaluationResult {
    pub passed: bool,
    pub issues: Vec<EvaluationIssue>,
}

#[derive(Debug, Clone)]
pub struct EvaluationIssue {
    pub file: PathBuf,
    pub line: usize,
    pub pattern: String,
    pub context: String,
}

impl std::fmt::Display for EvaluationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.passed {
            write!(f, "Evaluation passed: no stub patterns detected")
        } else {
            writeln!(f, "Evaluation FAILED: {} stub patterns detected:", self.issues.len())?;
            for issue in &self.issues {
                writeln!(
                    f,
                    "  {}:{} — found `{}` in: {}",
                    issue.file.display(),
                    issue.line,
                    issue.pattern,
                    issue.context.trim()
                )?;
            }
            Ok(())
        }
    }
}

/// Evaluate recently-modified files for stub patterns. Uses `git diff --name-only`
/// against HEAD to find modified files, then scans them for stub indicators.
///
/// Returns `EvaluationResult` with pass/fail and specific issues found.
pub fn evaluate_work_quality(git_root: &Path) -> EvaluationResult {
    let modified_files = match std::process::Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .current_dir(git_root)
        .output()
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|l| {
                    l.ends_with(".rs")
                        || l.ends_with(".ts")
                        || l.ends_with(".py")
                        || l.ends_with(".go")
                        || l.ends_with(".js")
                })
                .map(|l| git_root.join(l))
                .collect::<Vec<_>>()
        }
        _ => {
            // Also check staged files
            match std::process::Command::new("git")
                .args(["diff", "--name-only", "--cached"])
                .current_dir(git_root)
                .output()
            {
                Ok(output) if output.status.success() => {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .filter(|l| {
                            l.ends_with(".rs")
                                || l.ends_with(".ts")
                                || l.ends_with(".py")
                                || l.ends_with(".go")
                                || l.ends_with(".js")
                        })
                        .map(|l| git_root.join(l))
                        .collect::<Vec<_>>()
                }
                _ => return EvaluationResult { passed: true, issues: vec![] },
            }
        }
    };

    let mut issues = Vec::new();
    for file in &modified_files {
        let Ok(content) = std::fs::read_to_string(file) else {
            continue;
        };
        for (line_num, line) in content.lines().enumerate() {
            for pattern in STUB_PATTERNS {
                if line.contains(pattern) {
                    issues.push(EvaluationIssue {
                        file: file.clone(),
                        line: line_num + 1,
                        pattern: pattern.to_string(),
                        context: line.to_string(),
                    });
                }
            }
        }
    }

    EvaluationResult {
        passed: issues.is_empty(),
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprint_budget_low_utilization_no_prompt() {
        let budget = SprintBudget::compute(100_000, 1_000_000);
        assert_eq!(budget.utilization, 0.1);
        assert!(!budget.should_handoff);
        assert!(budget.system_prompt_section().is_none());
    }

    #[test]
    fn sprint_budget_warn_threshold() {
        let budget = SprintBudget::compute(750_000, 1_000_000);
        assert!(budget.utilization >= SPRINT_WARN_PCT);
        assert!(!budget.should_handoff);
        assert!(budget.system_prompt_section().is_some());
        assert!(budget.status_line.contains("wrap up"));
    }

    #[test]
    fn sprint_budget_handoff_threshold() {
        let budget = SprintBudget::compute(900_000, 1_000_000);
        assert!(budget.should_handoff);
        assert!(budget.status_line.contains("STOP"));
        assert!(budget.system_prompt_section().is_some());
    }

    #[test]
    fn handoff_summary_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let summary = HandoffSummary {
            timestamp: "2026-05-19T12:00:00Z".to_string(),
            completed_work: vec!["Implemented task persistence".to_string()],
            in_progress: vec!["Sprint boundary detection".to_string()],
            remaining_tasks: vec!["Evaluator pass".to_string()],
            decisions: vec!["Use project-level .jfc/tasks.json".to_string()],
            blockers: vec![],
        };
        let path = summary.write_to_disk(dir.path()).unwrap();
        assert!(path.exists());

        let content = HandoffSummary::read_latest(dir.path()).unwrap();
        assert!(content.contains("task persistence"));
        assert!(content.contains("Sprint boundary"));
    }
}
